import type { QueryClient, QueryKey } from "@tanstack/react-query";
import { ApiError } from "../../lib/api/ApiError";

/** Repeats after the first failure: about 1 s, 2 s and 4 s apart. */
export const QUICK_ACCESS_READ_RETRIES = 3;
/** The longest a server's Retry-After may hold the next attempt. */
const QUICK_ACCESS_MAX_RETRY_DELAY_MS = 8_000;

/**
 * Whether a failed quick-access read is worth repeating (TanStack counts
 * failures from 0). Transient failures:
 * - no connection (status 0)
 * - timeout 408, rate limit 429, server or gateway errors (5xx)
 * - the launch auth race: requestJson refreshes an expired token itself, but
 *   a read that lands while that refresh is still settling fails with 401 or
 *   `auth_session_changed`, and succeeds with the new token on the next try.
 * Validation and permission refusals (other 4xx), cancellations and an
 * unreadable answer are final. A sign-out or account switch is handled by the
 * caller, which re-keys the query and stops retrying the old one.
 */
export function retryQuickAccessRead(
  failureCount: number,
  error: unknown,
): boolean {
  if (failureCount >= QUICK_ACCESS_READ_RETRIES) return false;
  if (error instanceof ApiError) {
    const status = error.statusCode;
    return (
      status === 0 ||
      status === 401 ||
      status === 408 ||
      status === 429 ||
      status >= 500
    );
  }
  return error instanceof Error && error.message === "auth_session_changed";
}

/** Exponential backoff (1 s, 2 s, 4 s); a server's Retry-After can lengthen it. */
export function quickAccessRetryDelay(
  failureCount: number,
  error: unknown,
): number {
  const backoff = Math.min(1000 * 2 ** failureCount, 4000);
  const requested = error instanceof ApiError ? (error.retryAfterMs ?? 0) : 0;
  return Math.min(Math.max(backoff, requested), QUICK_ACCESS_MAX_RETRY_DELAY_MS);
}

/**
 * Read `key` again if its last read failed or is older than `staleMs`
 * (React Native has no window focus for TanStack to watch, so the rail's
 * hooks call this when the app returns to the foreground). A read already
 * running is not repeated. Returns whether a read was started.
 */
export function refetchIfDue(
  client: QueryClient,
  key: QueryKey,
  staleMs: number,
  now = Date.now(),
): boolean {
  const state = client.getQueryState(key);
  if (!state || state.fetchStatus !== "idle") return false;
  const due =
    state.status === "error" ||
    state.isInvalidated ||
    state.dataUpdatedAt === 0 ||
    now - state.dataUpdatedAt >= staleMs;
  if (due) void client.refetchQueries({ queryKey: key, exact: true });
  return due;
}
