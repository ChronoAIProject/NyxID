import { ApiError } from "@/lib/api-client";

export function isTerminalAuthFailureStatus(
  status: string | undefined,
): boolean {
  return status === "revoked" || status === "failed" || status === "expired";
}

/**
 * Consecutive poll-error count after which we give up and surface a
 * terminal failure to the user. With the default 2s interval that's
 * ~10s of "no usable response" — long enough to ride out a transient
 * 5xx or refresh-token churn, short enough that a dead wizard server
 * (issue #653 stale-CLI path) doesn't leave the user staring at
 * "Waiting…" indefinitely.
 */
const MAX_CONSECUTIVE_POLL_ERRORS = 5;

interface PollOAuthKeyUntilActiveOptions {
  readonly keyId: string;
  readonly getKey: (keyId: string) => Promise<{
    readonly status: string;
    readonly error_message?: string | null;
  }>;
  readonly completeWithKey: (keyId: string) => Promise<void>;
  readonly isCancelled: () => boolean;
  readonly onTerminalFailure: (key: {
    readonly status: string;
    readonly error_message?: string | null;
  }) => void;
  readonly onTimeout: () => void;
  readonly sleepMs?: (ms: number) => Promise<void>;
  readonly nowMs?: () => number;
  readonly timeoutMs?: number;
  readonly intervalMs?: number;
  readonly maxConsecutiveErrors?: number;
}

export async function pollOAuthKeyUntilActive({
  keyId,
  getKey,
  completeWithKey,
  isCancelled,
  onTerminalFailure,
  onTimeout,
  sleepMs = sleep,
  nowMs = Date.now,
  timeoutMs = 5 * 60 * 1000,
  intervalMs = 2000,
  maxConsecutiveErrors = MAX_CONSECUTIVE_POLL_ERRORS,
}: PollOAuthKeyUntilActiveOptions): Promise<void> {
  const deadline = nowMs() + timeoutMs;
  let consecutiveErrors = 0;
  while (nowMs() < deadline) {
    if (isCancelled()) return;
    await sleepMs(intervalMs);
    if (isCancelled()) return;
    try {
      const key = await getKey(keyId);
      consecutiveErrors = 0;
      if (key.status === "active") {
        await completeWithKey(keyId);
        return;
      }
      // Terminal failure statuses let provider denials and callback errors
      // exit immediately instead of waiting for the 5-minute deadline.
      if (isTerminalAuthFailureStatus(key.status)) {
        if (!isCancelled()) {
          onTerminalFailure(key);
        }
        return;
      }
    } catch (e) {
      // 404 means the placeholder is gone (abandoned by another tab,
      // hard-deleted, or never made it past the create response). Treat
      // it as terminal immediately so the wizard exits with a clear
      // message instead of polling silently for 5 minutes (issue #653
      // stale-tab path).
      if (e instanceof ApiError && e.status === 404) {
        if (!isCancelled()) {
          onTerminalFailure({
            status: "failed",
            error_message:
              "Authorization placeholder no longer exists. Cancel and re-run the wizard to try again.",
          });
        }
        return;
      }
      // Other errors (network, 5xx, refresh-token churn) are transient
      // when sporadic — but `MAX_CONSECUTIVE_POLL_ERRORS` in a row means
      // the wizard has lost contact with its server (CLI exited) or the
      // backend is unreachable for an extended window. Either way,
      // hanging on "Waiting…" is wrong; surface a terminal "lost
      // contact" failure so the user can act (issue #653 — wizard MUST
      // reach a terminal state).
      consecutiveErrors += 1;
      if (consecutiveErrors >= maxConsecutiveErrors) {
        if (!isCancelled()) {
          onTerminalFailure({
            status: "failed",
            error_message:
              "Lost contact with the wizard. Authorization may have completed — run `nyxid status` to verify, then cancel and re-run the wizard if the service is missing.",
          });
        }
        return;
      }
      // Still within tolerance window; keep polling.
    }
  }
  if (!isCancelled()) {
    onTimeout();
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}
