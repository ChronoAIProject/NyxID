/**
 * Background watch policy for an assistant connect card whose authorization
 * finishes out of band (OAuth / device-code completing in another tab).
 *
 * The CLI settles this with `wait_for_authorized_key`: a blocking loop that
 * polls `/keys/{id}` until `active` or `failed`. The CLI wizard settles it
 * with a local axum sidecar that sniffs the same responses, heartbeats the
 * page, and gives up on a ceiling. Both share one rule — the verdict belongs
 * to the server row, never to a UI callback — and both can hold that rule
 * because they run outside the page.
 *
 * The browser has no process that outlives the tab, so the two things the
 * sidecar owns are modelled here instead:
 *
 *   - liveness → anything the user does in the chat re-arms the window
 *     (the analogue of `HEARTBEAT_DEAD_AFTER`, but additive rather than
 *     a kill switch: chatting about something else is presence, not
 *     abandonment)
 *   - ceiling → a hard stop so a walked-away tab eventually gives up and
 *     says so (the analogue of `WIZARD_MAX_DURATION`)
 *
 * Everything here is pure or module-local so the policy is unit-testable
 * without mounting the chat.
 */

/** Base window. Long enough for a real OAuth detour incl. 2FA + account switch. */
export const CONNECT_WATCH_BASE_MS = 10 * 60_000;
/** Each chat interaction re-arms the deadline to at least this far out. */
export const CONNECT_WATCH_ACTIVITY_GRACE_MS = 5 * 60_000;
/** Hard ceiling regardless of activity. Mirrors the wizard's 30-minute cap. */
export const CONNECT_WATCH_MAX_MS = 30 * 60_000;
/** Tight cadence while the user is plausibly still on the provider's page. */
export const CONNECT_WATCH_FAST_INTERVAL_MS = 2_000;
export const CONNECT_WATCH_FAST_WINDOW_MS = 60_000;
/** Background cadence afterwards. Window focus still resolves instantly. */
export const CONNECT_WATCH_SLOW_INTERVAL_MS = 10_000;

/**
 * When this watch should give up, given when it started and when the user was
 * last active in the chat. Activity can only ever push the deadline out, and
 * never past the ceiling.
 */
export function connectWatchDeadline(
  startedAt: number,
  lastActivityAt: number,
): number {
  const base = startedAt + CONNECT_WATCH_BASE_MS;
  const extended =
    lastActivityAt > startedAt
      ? lastActivityAt + CONNECT_WATCH_ACTIVITY_GRACE_MS
      : base;
  return Math.min(Math.max(base, extended), startedAt + CONNECT_WATCH_MAX_MS);
}

/** Poll cadence: fast for the first minute, background thereafter. */
export function connectWatchInterval(startedAt: number, now: number): number {
  return now - startedAt > CONNECT_WATCH_FAST_WINDOW_MS
    ? CONNECT_WATCH_SLOW_INTERVAL_MS
    : CONNECT_WATCH_FAST_INTERVAL_MS;
}

// --- Chat activity: the browser's stand-in for the wizard's heartbeat ---
//
// Module-local rather than a store because there is exactly one chat surface
// per tab and every consumer wants the same scalar. Kept out of React state
// so non-component code (transport, mutations) can mark activity too.

let lastChatActivityAt = 0;
const activityListeners = new Set<() => void>();

/**
 * Record that the user did something in the chat — sent a message, decided an
 * approval, switched conversation. Any of those prove they are still present,
 * which is what extends a pending connect watch.
 */
export function markChatActivity(now: number = Date.now()): void {
  if (now <= lastChatActivityAt) return;
  lastChatActivityAt = now;
  for (const listener of activityListeners) listener();
}

export function getLastChatActivityAt(): number {
  return lastChatActivityAt;
}

export function subscribeChatActivity(listener: () => void): () => void {
  activityListeners.add(listener);
  return () => {
    activityListeners.delete(listener);
  };
}

export function resetChatActivityForTests(): void {
  lastChatActivityAt = 0;
  activityListeners.clear();
}
