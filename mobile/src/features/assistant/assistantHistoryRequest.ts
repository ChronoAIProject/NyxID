import { HISTORY_UNAVAILABLE } from "./assistantHistoryCompat";

export const ASSISTANT_HISTORY_TIMEOUT_MS = 10_000;

/** Bound the complete read, including transports or response bodies ignoring abort. */
export function withAssistantHistoryDeadline<T>(
  controller: AbortController,
  read: () => Promise<T>,
): Promise<T> {
  const cancelled = () =>
    Object.assign(new Error("Request cancelled"), { name: "AbortError" });
  if (controller.signal.aborted) return Promise.reject(cancelled());

  let interrupt!: (error: Error) => void;
  const interrupted = new Promise<never>((_, reject) => {
    interrupt = reject;
  });
  const abort = () => interrupt(cancelled());
  controller.signal.addEventListener("abort", abort, { once: true });
  const timer = setTimeout(() => {
    const error = Object.assign(new Error(HISTORY_UNAVAILABLE), {
      name: "HistoryRequestTimeoutError",
    });
    // Preserve the timeout reason when abort synchronously rejects the transport.
    interrupt(error);
    controller.abort();
  }, ASSISTANT_HISTORY_TIMEOUT_MS);

  let request: Promise<T>;
  try {
    request = read();
  } catch (error) {
    request = Promise.reject(error);
  }
  return Promise.race([interrupted, request]).finally(() => {
    clearTimeout(timer);
    controller.signal.removeEventListener("abort", abort);
  });
}
