export const APPROVAL_REFRESH_THROTTLE_MS = 1_000;
// Push signals are the primary path. Focused approval surfaces poll only as a
// recovery bound for notifications the OS delays or drops entirely.
export const APPROVAL_BACKSTOP_POLL_INTERVAL_MS = 30_000;

type ApprovalRefreshListener = () => void;

type ApprovalRefreshSignalOptions = {
  throttleMs?: number;
  now?: () => number;
  setTimer?: (callback: () => void, delayMs: number) => unknown;
  clearTimer?: (timer: unknown) => void;
};

export type ApprovalRefreshSignal = {
  signal: () => void;
  subscribe: (listener: ApprovalRefreshListener) => () => void;
  clear: () => void;
};

/**
 * Creates a leading-edge throttle with one trailing catch-up delivery.
 *
 * A pending signal survives periods with no subscribers, but no timer does.
 * That lets a focused screen catch up immediately after the app resumes or
 * the route regains focus without keeping background JS work alive.
 */
export function createApprovalRefreshSignal(
  options: ApprovalRefreshSignalOptions = {}
): ApprovalRefreshSignal {
  const throttleMs = options.throttleMs ?? APPROVAL_REFRESH_THROTTLE_MS;
  const now = options.now ?? Date.now;
  const setTimer =
    options.setTimer ??
    ((callback: () => void, delayMs: number) => setTimeout(callback, delayMs));
  const clearTimer =
    options.clearTimer ??
    ((timer: unknown) => clearTimeout(timer as ReturnType<typeof setTimeout>));

  const listeners = new Set<ApprovalRefreshListener>();
  let lastDeliveredAt = Number.NEGATIVE_INFINITY;
  let pending = false;
  let trailingTimer: unknown | null = null;

  const cancelTrailingTimer = () => {
    if (trailingTimer === null) return;
    clearTimer(trailingTimer);
    trailingTimer = null;
  };

  const deliver = () => {
    if (!pending || listeners.size === 0) return;

    pending = false;
    lastDeliveredAt = now();
    for (const listener of [...listeners]) {
      try {
        listener();
      } catch {
        // A route owns its refresh side effect. One broken owner must not
        // prevent the other focused approval surface from seeing the signal.
      }
    }
  };

  const deliverOrSchedule = () => {
    if (!pending || listeners.size === 0 || trailingTimer !== null) return;

    const remainingMs = Math.max(0, throttleMs - (now() - lastDeliveredAt));
    if (remainingMs === 0) {
      deliver();
      return;
    }

    trailingTimer = setTimer(() => {
      trailingTimer = null;
      deliver();
    }, remainingMs);
  };

  return {
    signal() {
      pending = true;
      deliverOrSchedule();
    },
    subscribe(listener) {
      listeners.add(listener);
      deliverOrSchedule();

      return () => {
        listeners.delete(listener);
        if (listeners.size === 0) {
          // Preserve `pending` for the next active subscriber, but never let
          // a route blur or app background leave a throttle timer behind.
          cancelTrailingTimer();
        }
      };
    },
    clear() {
      cancelTrailingTimer();
      pending = false;
      // `clear` is an auth/session boundary, not ordinary timer cleanup. The
      // next user's first signal must get a fresh leading-edge delivery even
      // if the previous user received one less than a throttle window ago.
      lastDeliveredAt = Number.NEGATIVE_INFINITY;
    },
  };
}

const approvalRefreshSignal = createApprovalRefreshSignal();

export function signalApprovalStateMayHaveChanged(): void {
  approvalRefreshSignal.signal();
}

export function subscribeToApprovalRefreshSignals(
  listener: ApprovalRefreshListener
): () => void {
  return approvalRefreshSignal.subscribe(listener);
}

export function clearPendingApprovalRefreshSignal(): void {
  approvalRefreshSignal.clear();
}
