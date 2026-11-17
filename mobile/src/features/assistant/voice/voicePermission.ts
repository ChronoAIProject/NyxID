import { localizedError } from "../../../i18n/localizedError";
type ForegroundApp = {
  readonly currentState: string;
  addEventListener(
    event: "change",
    listener: (state: string) => void,
  ): { remove(): void };
};

export const VOICE_FOREGROUND_TIMEOUT_MS = 5000;

/** Permission results can arrive before the native permission activity has resumed NyxID. */
export function waitForVoiceForeground(
  AppState: ForegroundApp,
  signal: AbortSignal,
  valid: () => boolean,
  options: {
    allowInitialBackground?: boolean;
    waitWhileInactive?: boolean;
  } = {},
) {
  if (signal.aborted || !valid())
    return Promise.reject(localizedError("Voice start was cancelled."));
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
    const aborted = () => finish(localizedError("Voice start was cancelled."));
    const changed = (state: string, initial = false) => {
      if (signal.aborted || !valid()) aborted();
      else if (state === "active") finish();
      else if (
        state === "background" &&
        !(initial && options.allowInitialBackground)
      )
        finish(localizedError("Keep NyxID open to start a voice call."));
      else if (state === "inactive" && options.waitWhileInactive) {
        // iOS can keep an alert/overlay up indefinitely without backgrounding.
        clearTimeout(timeout);
        timeout = undefined;
      } else if (!timeout)
        timeout = setTimeout(
          () =>
            finish(
              localizedError("NyxID did not return to the foreground. Open the app and try voice again."),
            ),
          VOICE_FOREGROUND_TIMEOUT_MS,
        );
    };
    signal.addEventListener("abort", aborted, { once: true });
    subscription = AppState.addEventListener("change", changed);
    // Cover a state or abort transition between the first check and subscribing.
    changed(AppState.currentState, true);
  });
}
