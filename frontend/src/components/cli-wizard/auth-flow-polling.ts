import { ApiError } from "@/lib/api-client";

export function isTerminalAuthFailureStatus(
  status: string | undefined,
): boolean {
  return status === "revoked" || status === "failed" || status === "expired";
}

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
}: PollOAuthKeyUntilActiveOptions): Promise<void> {
  const deadline = nowMs() + timeoutMs;
  while (nowMs() < deadline) {
    if (isCancelled()) return;
    await sleepMs(intervalMs);
    if (isCancelled()) return;
    try {
      const key = await getKey(keyId);
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
      // it as terminal so the wizard exits with a clear message instead
      // of polling silently for 5 minutes (issue #653 stale-tab path).
      // All other errors (network, 5xx, refresh-token churn) stay
      // transient — keep polling.
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
      // Transient; keep polling.
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
