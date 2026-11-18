export const VOICE_RECOVERY_TIMEOUT_MS = 8_000;

function abortError() {
  const error = new Error("Voice request was cancelled");
  error.name = "AbortError";
  return error;
}

export function withVoiceRequestDeadline<T>(
  controller: AbortController,
  operation: () => Promise<T>,
  timeoutMs = VOICE_RECOVERY_TIMEOUT_MS,
): Promise<T> {
  if (controller.signal.aborted) return Promise.reject(abortError());

  let rejectInterrupted!: (error: Error) => void;
  const interrupted = new Promise<never>((_, reject) => {
    rejectInterrupted = reject;
  });
  const onAbort = () => rejectInterrupted(abortError());
  controller.signal.addEventListener("abort", onAbort, { once: true });
  const timer = setTimeout(() => {
    const error = new Error("Voice recovery request timed out");
    error.name = "VoiceRequestTimeoutError";
    // Reject first so the abort notification cannot replace the timeout error.
    rejectInterrupted(error);
    controller.abort(error);
  }, timeoutMs);

  let request: Promise<T>;
  try {
    request = operation();
  } catch (error) {
    request = Promise.reject(error);
  }

  // The race also bounds transports that do not honor their AbortSignal.
  return Promise.race([interrupted, request]).finally(() => {
    clearTimeout(timer);
    controller.signal.removeEventListener("abort", onAbort);
  });
}
