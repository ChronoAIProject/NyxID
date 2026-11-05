import { getAuthEpoch } from "../auth/authEpoch";

export function createConnectionState() {
  let lost = false;
  let sequence = 0;
  let settled = 0;
  const listeners = new Set<() => void>();
  const set = (value: boolean) => {
    if (lost !== value) {
      lost = value;
      for (const listener of listeners) listener();
    }
  };
  return {
    snapshot: () => lost,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    async observe<T>(send: () => Promise<T>, signal?: AbortSignal) {
      const request = ++sequence;
      const epoch = getAuthEpoch();
      const current = () =>
        !signal?.aborted && epoch === getAuthEpoch() && request >= settled;
      try {
        const value = await send();
        if (current()) {
          settled = request;
          set(false);
        }
        return value;
      } catch (cause) {
        if (
          current() &&
          !(cause instanceof Error && cause.name === "AbortError")
        ) {
          settled = request;
          set(true);
        }
        throw cause;
      }
    },
  };
}
export const assistantConnection = createConnectionState();
