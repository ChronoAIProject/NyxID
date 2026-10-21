/** Product events use the authenticated NyxID relay and its live consent gate.
 * No vendor SDK, replay, autocapture, anonymous identity or persistent event queue.
 */
import { apiClient } from './api-client';
import { useAuthStore } from '@/stores/auth-store';
import type { UiEvent } from './telemetry-schema';
import { useConsentStore } from '@/stores/consent-store';

let configured = false;
let actorId: string | null = null;
let generation = 0;
let admissions = 0;
const MAX_ADMISSIONS = 16;

function isDntActive(): boolean {
  if (typeof navigator === "undefined") return false;
  // Modern browsers: navigator.doNotTrack
  const nav = (navigator as Navigator & { msDoNotTrack?: string }).doNotTrack;
  if (nav === "1" || nav === "yes") return true;
  // Legacy IE/old Edge: navigator.msDoNotTrack
  const ms = (navigator as Navigator & { msDoNotTrack?: string }).msDoNotTrack;
  if (ms === "1") return true;
  // Non-standard Firefox-era fallback
  const win = (
    typeof window !== "undefined" ? (window as Window & { doNotTrack?: string }) : null
  )?.doNotTrack;
  if (win === "1" || win === "yes") return true;
  return false;
}

const SENSITIVE_PATH_PATTERNS: RegExp[] = [
  /^\/nyxbot\/link(?:\/|$)/,
  /^\/onboarding\/link(?:\/|$)/,
  /^\/connect\/bot\/[^/]+/,
  /^\/login\/(?:device|agent-key|code)(?:\/|$)/,
  /\/verify-email\/[^/]+/,
  /\/reset-password\/[^/]+/,
  /\/oauth\/callback/,
  /^\/oauth-complete$/,
  /\/approve\/[^/]+/,
];

function isSensitivePath(path: string): boolean {
  return SENSITIVE_PATH_PATTERNS.some((pattern) => pattern.test(path));
}

function isSensitiveCurrentPage(): boolean {
  return typeof window !== 'undefined' && isSensitivePath(window.location.pathname);
}

export function isTelemetryActive(): boolean {
  const device = useConsentStore.getState();
  return configured && actorId !== null && device.accountId === actorId && device.enabled
    && !isDntActive() && !isSensitiveCurrentPage();
}

export interface InitTelemetryArgs {
  dsn: string | null | undefined;
  host?: string | null;
  shareBack?: boolean;
  consent: boolean;
}

export function initTelemetry(args: InitTelemetryArgs): void {
  configured = args.consent && Boolean(args.dsn?.trim() || args.shareBack);
}

/** Local identity only; the server authenticates every admission and capture. */
export function identify(userId: string): void {
  if (actorId === userId) return;
  generation++;
  actorId = userId || null;
}

export function reset(): void {
  generation++;
  actorId = null;
  configured = false;
}

export function disableTelemetry(): void {
  generation++;
  configured = false;
}

/** Bounded ephemeral work. Every event obtains its own fresh consent revision. */
export function capture(event: UiEvent): Promise<void> {
  if (!isTelemetryActive() || admissions >= MAX_ADMISSIONS) return Promise.resolve();
  const accountId = actorId!;
  const epoch = generation;
  const deviceRevision = useConsentStore.getState().revision;
  admissions++;
  return (async () => {
    const guard = () => {
      if (generation !== epoch || useConsentStore.getState().revision !== deviceRevision || !isTelemetryActive() || actorId !== accountId
          || useAuthStore.getState().user?.id !== accountId) throw new Error('Analytics authority changed');
    };
    guard();
    const permission = await apiClient<{ account_id: string; enabled: boolean; revision: string }>(
      '/users/me/preferences/analytics', { authorityGuard: guard },
    );
    guard();
    if (!permission.enabled || permission.account_id !== accountId || !permission.revision) return;
    await apiClient('/users/me/analytics/events', {
      method: 'POST', authorityGuard: guard,
      body: { account_id: accountId, revision: permission.revision, event },
    });
  })().catch(() => {
    // Analytics is optional. Never fall back to direct vendor delivery.
  }).finally(() => { admissions--; });
}

/** Error text and stacks may contain credentials; no raw exception forwarding. */
export function captureException(err: unknown): void {
  void err;
  // Operational/security errors remain in their dedicated reporting paths.
}
