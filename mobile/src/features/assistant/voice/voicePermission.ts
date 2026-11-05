type ForegroundApp = {
  readonly currentState: string;
  addEventListener(
    event: "change",
    listener: (state: string) => void,
  ): { remove(): void };
};

/** iOS permission can resolve before its transient inactive state has ended. */
export function waitForVoiceForeground(
  AppState: ForegroundApp,
  signal: AbortSignal,
  valid: () => boolean,
) {
  if (signal.aborted || !valid())
    return Promise.reject(new Error("Voice start was cancelled."));
  if (AppState.currentState === "active") return Promise.resolve();
  return new Promise<void>((resolve, reject) => {
    let settled = false;
    let subscription: ReturnType<typeof AppState.addEventListener> | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      subscription?.remove();
      clearTimeout(timeout);
      signal.removeEventListener("abort", aborted);
      if (error) reject(error);
      else resolve();
    };
    const aborted = () => finish(new Error("Voice start was cancelled."));
    const changed = (state: string) => {
      if (signal.aborted || !valid()) aborted();
      else if (state === "active") finish();
      else if (state === "background")
        finish(new Error("Keep NyxID open to start a voice call."));
    };
    signal.addEventListener("abort", aborted, { once: true });
    subscription = AppState.addEventListener("change", changed);
    timeout = setTimeout(
      () =>
        finish(
          new Error(
            "NyxID did not return to the foreground. Open the app and try voice again.",
          ),
        ),
      5000,
    );
    // Cover a state or abort transition between the first check and subscribing.
    changed(AppState.currentState);
  });
}
