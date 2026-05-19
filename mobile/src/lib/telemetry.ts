/**
 * Mobile telemetry — vendor-neutral public API.
 *
 * Public surface mirrors frontend/src/lib/telemetry.ts: `init`,
 * `identify`, `reset`, `capture`, `captureException`. The
 * `posthog-react-native` SDK is imported only here; no caller
 * references it. Swapping vendors = rewriting this file (§5.0
 * hot-swap contract in `docs/TELEMETRY.md`).
 *
 * Mobile-specific hardening (§7.3):
 *   - `enableSessionReplay: false` (never on for an auth app)
 *   - `customAppProperties` strips `$device_name` (often contains the
 *     user's first name on iOS) and `$device_id` (hardware-identifying)
 *   - `beforeSend` drops deep-link URLs that embed tokens
 *   - Push notification body is NEVER captured — only `type` + `app_state`
 */

import Constants from 'expo-constants';
import { PostHog } from 'posthog-react-native';

import type { MobileEvent } from './telemetry-schema';

// JsonType-compatible shape — matches @posthog/core's PostHogEventProperties.
// Re-defined locally to avoid importing a deep-path type that may move
// across minor versions of posthog-react-native.
type PostHogPropertyBag = Record<string, unknown>;

let client: PostHog | null = null;
let inited = false;

// --- Privacy helpers -------------------------------------------------

/**
 * Deep-link URL patterns that embed a sensitive token (challenge id,
 * approval id). Events whose `url`/`$current_url` matches one of these
 * are dropped entirely by `beforeSend` — we still emit
 * `mobile.deep_link_opened` with the narrow `link_type` enum, never
 * the raw URL.
 */
const SENSITIVE_DEEPLINK_PATTERNS: RegExp[] = [
  /nyxid:\/\/challenge\/[^/?]+/,
  /nyxid:\/\/approve\/[^/?]+/,
];

function stripQueryString(v: unknown): unknown {
  if (typeof v !== 'string') return v;
  const idx = v.indexOf('?');
  return idx >= 0 ? v.slice(0, idx) : v;
}

// --- Public API ------------------------------------------------------

export interface InitMobileTelemetryArgs {
  dsn: string | undefined;
  host: string | undefined;
  shareBack?: boolean;
  consent: boolean;
}

/**
 * Idempotent init. No-op when consent is false, DSN is empty, or
 * already inited. Safe to call from a `useEffect` that fires more
 * than once.
 */
/**
 * Compiled-in share-back DSN. Empty until a release process bakes in
 * the real value. Matches backend/CLI/frontend wrapper pattern.
 */
const NYXID_PUBLIC_TELEMETRY_DSN = 'phc_pHHMZRXY8ymzBy9uwiGmAVDtGvGpDTiyXH2zs7bQWEgM';
const NYXID_PUBLIC_TELEMETRY_HOST = 'https://us.i.posthog.com';

export async function initTelemetry(args: InitMobileTelemetryArgs): Promise<void> {
  if (inited) return;
  if (!args.consent) return;

  // Precedence (docs/TELEMETRY.md §3): explicit DSN wins, then
  // share-back falls back to the compiled-in public DSN (currently
  // empty, so share-back silently degrades to "off" until release).
  let dsn = (args.dsn ?? '').trim();
  let host = (args.host ?? '').trim();
  if (!dsn && args.shareBack && NYXID_PUBLIC_TELEMETRY_DSN.length > 0) {
    dsn = NYXID_PUBLIC_TELEMETRY_DSN;
    if (!host) host = NYXID_PUBLIC_TELEMETRY_HOST;
  }
  if (!dsn) return;
  if (!host) host = 'https://us.i.posthog.com';

  const ph = new PostHog(dsn, {
    host,
    captureAppLifecycleEvents: true,
    enableSessionReplay: false,
    // Strip identifying device props before any event is built.
    // `$device_name` often carries the user's first name on iOS;
    // `$device_id` is a persistent hardware identifier the privacy
    // contract in this file (and docs/TELEMETRY.md §6) requires us to
    // drop. `$device_model` + OS + app_version is plenty for debugging.
    // Cast to an index-signature shape since PostHog's typed prop bag
    // doesn't expose `$device_id` in the `customAppProperties` argument.
    customAppProperties: (props) => {
      const out: Record<string, unknown> = { ...props };
      out.$device_name = null;
      out.$device_id = null;
      return out as typeof props;
    },
    // Drop deep-link events that embed a token in the URL. We still
    // emit `mobile.deep_link_opened` with a narrow `link_type` enum
    // separately, so the signal survives even when the raw URL is gone.
    before_send: (event) => {
      if (!event) return null;

      const props = event.properties as Record<string, unknown> | undefined;
      if (props) {
        for (const key of ['$current_url', '$referrer', 'url']) {
          if (key in props) {
            props[key] = stripQueryString(props[key]);
          }
        }
        const maybeUrl =
          (typeof props.url === 'string' && props.url) ||
          (typeof props.$current_url === 'string' && props.$current_url) ||
          '';
        if (maybeUrl) {
          for (const re of SENSITIVE_DEEPLINK_PATTERNS) {
            if (re.test(maybeUrl)) return null;
          }
        }
      }
      return event;
    },
  });

  // Ensure opt-in: if the user previously opted out via
  // `shutdownTelemetry()`, the SDK persisted that flag to storage.
  // Calling `optIn()` on the freshly-constructed client clears it so
  // events flow again after re-consent.
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const asAny = ph as any;
    if (typeof asAny.optIn === "function") {
      asAny.optIn();
    }
  } catch {
    // Best-effort.
  }

  client = ph;
  inited = true;
}

/**
 * Associate the current session with the authenticated NyxID user.
 * Delegates to the vendor's merge verb (`identify`). No-op when
 * telemetry is off.
 */
export function identify(userId: string): void {
  if (!inited || !client) return;
  if (!userId) return;
  client.identify(userId);
}

/**
 * Clear the local identity and assign a fresh anon distinct_id. Call
 * from EVERY sign-out path: explicit sign-out, 401 session
 * invalidation, SecureStore wipe on auth failure, account switch.
 */
export function reset(): void {
  if (!inited || !client) return;
  client.reset();
}

/**
 * Emit a named event. Prop shape is type-checked against the schema
 * in `telemetry-schema.ts`; unknown event names are a compile error.
 */
export function capture(event: MobileEvent): void {
  if (!inited || !client) return;
  // Cast via unknown: the schema's discriminated union maps 1:1 onto
  // the vendor's property bag. The narrow compile-time type was the
  // whole point of the schema; at the vendor boundary we loosen once.
  const props = event.props as unknown as PostHogPropertyBag;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (client as any).capture(event.name, props);
}

export function captureException(err: unknown): void {
  if (!inited || !client) return;
  client.captureException(err);
}

/**
 * Fully tear down the telemetry client. Unlike `reset()` (which only
 * clears the current identity), this clears the local identity, releases
 * the vendor client, and flips `inited` back to `false` so subsequent
 * `capture()` / `identify()` / `captureException()` calls short-circuit
 * until `initTelemetry()` is called again.
 *
 * Call from the consent-revoke path: when a user turns analytics off in
 * Settings, this ensures no further events reach the vendor -- satisfies
 * the privacy-policy promise that turning analytics off "stops new
 * events immediately."
 */
export function shutdownTelemetry(): void {
  if (!inited) return;
  const snapshot = client;
  // Flip our gate first so new calls into `capture()` short-circuit
  // even if the vendor teardown below takes a moment.
  client = null;
  inited = false;
  if (!snapshot) return;
  try {
    // `optOut()` is the SDK-level opt-out. It persists the flag to
    // storage AND suppresses any lifecycle / captureAppLifecycleEvents
    // listeners the SDK has already wired up. Just `reset()` on its own
    // only clears the current identity -- the vendor keeps emitting
    // $app_background / $app_foreground / etc. until process exit,
    // which would violate the "stops new events immediately" promise
    // in Settings.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const asAny = snapshot as any;
    if (typeof asAny.optOut === "function") {
      asAny.optOut();
    }
  } catch {
    // Best-effort; never throw out of a consent path.
  }
  try {
    snapshot.reset();
  } catch {
    // Best-effort; never throw out of a consent path.
  }
}

// Helper for app shell to read the expo-config DSN/host.
export function readExpoTelemetryConfig(): {
  dsn: string | undefined;
  host: string | undefined;
  shareBack: boolean;
} {
  const extra = (Constants.expoConfig?.extra ?? {}) as Record<string, unknown>;
  const dsn = typeof extra.TELEMETRY_DSN === 'string' ? extra.TELEMETRY_DSN : undefined;
  const host = typeof extra.TELEMETRY_HOST === 'string' ? extra.TELEMETRY_HOST : undefined;
  const shareBack = extra.NYXID_SHARE_ANALYTICS === 'true' || extra.NYXID_SHARE_ANALYTICS === true;
  return { dsn, host, shareBack };
}

