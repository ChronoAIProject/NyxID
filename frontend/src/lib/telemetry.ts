/**
 * Frontend telemetry — vendor-neutral public API.
 *
 * Public surface is the short verb list from `docs/TELEMETRY_M1.md`
 * §5.0 hot-swap contract: `initTelemetry / identify / reset / capture /
 * captureException`. The PostHog SDK (`posthog-js`) is imported only
 * here; no caller references it. Swapping vendors = rewriting this file.
 *
 * Privacy posture (§6):
 *   - `mask_all_text` + `mask_all_element_attributes` ON
 *   - CSS denylist covers password / secret / OTP / otp / credential inputs
 *   - `before_send` strips URL query strings (reset tokens, OAuth codes)
 *     and drops events on sensitive paths (/reset-password/*, /verify-email/*, etc.)
 *   - `ip: false`, `save_campaign_params: false`, `respect_dnt: true`
 *   - `capture_exceptions: true` for uncaught errors + unhandled rejections
 *   - `persistence: 'localStorage'` (survives reloads; cleared by `reset`)
 *
 * Consent: this module is a no-op unless `init` is called with
 * `consent: true`. The module-level `inited` guard also makes StrictMode
 * double-invoke safe.
 */

import posthog from 'posthog-js';

import type { UiEvent } from './telemetry-schema';

// --- Module-level state (StrictMode-idempotent) -----------------------

/**
 * Compiled-in share-back DSN. Left empty by default; the release
 * process is expected to bake in the real value before shipping a
 * version that advertises `NYXID_SHARE_ANALYTICS=true` support. A
 * zero-length value means share-back silently degrades to "off",
 * which is the safest possible default (parity with `cli/src/telemetry/mod.rs`
 * and `backend/src/telemetry/mod.rs`).
 */
const NYXID_PUBLIC_TELEMETRY_DSN = "";
const NYXID_PUBLIC_TELEMETRY_HOST = "https://eu.i.posthog.com";

let inited = false;

// --- Privacy config helpers -------------------------------------------

/**
 * CSS selectors covering elements that must never have their text or
 * attributes captured by analytics. This list is enforced structurally:
 * `mask_all_text` + `mask_all_element_attributes` are on globally, and
 * code reviewers check that any new sensitive input is tagged with
 * `data-sensitive` or one of the name patterns below.
 *
 * Exported so tests, reviewers, and any future client-side auditor can
 * diff the actual selector set used.
 */
export const AUTOCAPTURE_DENYLIST = [
  'input[type="password"]',
  'input[name*="password"]',
  'input[name*="secret"]',
  'input[name="code"][autocomplete="one-time-code"]',
  'input[name*="otp"]',
  '[data-sensitive]',
  '[data-api-key]',
  '[data-credential]',
] as const;

/**
 * Path patterns that must never produce a `$pageview` — the path itself
 * contains a sensitive token (reset code, verification code, OAuth
 * response) that would leak if captured.
 */
const SENSITIVE_PATH_PATTERNS: RegExp[] = [
  /\/verify-email\/[^/]+/,
  /\/reset-password\/[^/]+/,
  /\/oauth\/callback/,
  /\/approve\/[^/]+/,
];

function stripQueryString(url: string | undefined): string | undefined {
  if (!url) return url;
  const idx = url.indexOf('?');
  return idx >= 0 ? url.slice(0, idx) : url;
}

// --- Public API -------------------------------------------------------

export interface InitTelemetryArgs {
  /** Vendor DSN (e.g. PostHog project API key). Empty = no-op. */
  dsn: string | undefined;
  /** Ingest host; defaults to PostHog Cloud EU when omitted. */
  host: string | undefined;
  /** Community opt-in flag; currently unused at this layer because
   * `VITE_NYXID_SHARE_ANALYTICS` is read by the caller before deciding
   * which DSN to pass. Plumbed for future host/DSN swap logic. */
  shareBack?: boolean;
  /** User's consent state. When `false`, init does nothing. */
  consent: boolean;
}

/**
 * Idempotent init. No-op when:
 *   - `consent` is false,
 *   - `dsn` is empty,
 *   - already inited (StrictMode double-invoke safe).
 *
 * Callers must pass `consent: true` only after the user has opted in.
 */
export function initTelemetry(args: InitTelemetryArgs): void {
  if (inited) return;
  if (!args.consent) return;

  // Precedence (docs/TELEMETRY_M1.md §3): explicit DSN wins, then
  // share-back falls back to the compiled-in public DSN (currently
  // empty so share-back silently degrades to "off" until a release
  // bakes in the real value). Matches CLI + backend precedence.
  let dsn = (args.dsn ?? '').trim();
  let host = (args.host ?? '').trim();
  if (!dsn && args.shareBack && NYXID_PUBLIC_TELEMETRY_DSN.length > 0) {
    dsn = NYXID_PUBLIC_TELEMETRY_DSN;
    if (!host) host = NYXID_PUBLIC_TELEMETRY_HOST;
  }
  if (!dsn) return;
  if (!host) host = 'https://eu.i.posthog.com';

  posthog.init(dsn, {
    api_host: host,

    // --- Capture behavior ---
    // `mask_all_text` masks every captured innerText/value; posthog-js
    // already redacts `input[type="password"]` automatically. The
    // `AUTOCAPTURE_DENYLIST` list above is enforced at the DOM layer
    // (components annotate sensitive inputs with `data-sensitive` or
    // matching names) and referenced by code reviewers during audits.
    mask_all_text: true,
    mask_all_element_attributes: true,
    capture_pageview: true,
    capture_pageleave: true,
    capture_exceptions: true,

    // --- Privacy posture ---
    persistence: 'localStorage',
    opt_out_capturing_by_default: false, // consent is enforced at init time, not by this flag
    save_campaign_params: false,
    ip: false,
    respect_dnt: true,

    // --- Egress hook: last chance to drop / mutate before send ---
    before_send: (event) => {
      if (!event) return null;

      // Strip query strings from every captured URL.
      if (event.properties) {
        const props = event.properties as Record<string, unknown>;
        if (typeof props.$current_url === 'string') {
          props.$current_url = stripQueryString(props.$current_url);
        }
        if (typeof props.$referrer === 'string') {
          props.$referrer = stripQueryString(props.$referrer);
        }

        // Drop pageviews on sensitive paths.
        const pathname = typeof props.$pathname === 'string' ? props.$pathname : '';
        if (pathname) {
          for (const re of SENSITIVE_PATH_PATTERNS) {
            if (re.test(pathname)) {
              return null;
            }
          }
        }
      }

      return event;
    },
  });

  inited = true;
}

/**
 * Associate the current session with the authenticated NyxID user.
 * Wrapper for the vendor's merge verb. On PostHog this transparently
 * aliases the current anon distinct_id into `userId`, merging prior
 * pre-auth pageviews into the authenticated person record.
 *
 * No-op when telemetry is off — safe to call unconditionally from the
 * auth store's post-login hook.
 */
export function identify(userId: string): void {
  if (!inited) return;
  if (!userId) return;
  posthog.identify(userId);
}

/**
 * Clear the local identity and assign a fresh anon distinct_id. Call
 * from every sign-out path (explicit logout, session invalidation,
 * account switch). Preserves privacy across shared machines.
 *
 * No-op when telemetry is off.
 */
export function reset(): void {
  if (!inited) return;
  posthog.reset();
}

/**
 * Emit a named event. Prop shape is type-checked against the schema
 * in `telemetry-schema.ts`; unknown event names are a compile error.
 *
 * No-op when telemetry is off.
 */
export function capture(event: UiEvent): void {
  if (!inited) return;
  posthog.capture(event.name, event.props as Record<string, unknown>);
}

/**
 * Manually capture an exception (use for handled errors worth
 * surfacing in the issues dashboard). Uncaught errors + unhandled
 * rejections are auto-captured via `capture_exceptions: true`.
 *
 * No-op when telemetry is off.
 */
export function captureException(err: unknown): void {
  if (!inited) return;
  posthog.captureException?.(err as Error);
}

// --- For tests -------------------------------------------------------

/** Reset the internal `inited` flag. Test-only; do not call from app code. */
export function __resetForTests(): void {
  inited = false;
}
