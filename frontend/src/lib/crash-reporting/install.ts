import { reportCrash } from "./reporter";
/** Listeners observe without suppressing browser or React fallback behavior. */
export function installCrashReporting(): () => void {
  const error = (event: ErrorEvent | Event) => {
    if (event instanceof ErrorEvent) void reportCrash(event.error, "js_error");
    else void reportCrash(undefined, "resource_error");
  };
  const rejection = (event: PromiseRejectionEvent) => {
    void reportCrash(event.reason, "unhandled_rejection");
  };
  window.addEventListener("error", error, true);
  window.addEventListener("unhandledrejection", rejection);
  return () => {
    window.removeEventListener("error", error, true);
    window.removeEventListener("unhandledrejection", rejection);
  };
}
function reportToBrowser(error: unknown): void {
  // React's documented default reports to window.reportError. Older browsers
  // retain their uncaught-error path through an asynchronous throw.
  if (typeof window.reportError === "function") window.reportError(error);
  else
    window.setTimeout(() => {
      throw error;
    }, 0);
}
export const crashRootOptions = {
  onUncaughtError(error: unknown) {
    void reportCrash(error, "js_error");
    reportToBrowser(error);
  },
  onCaughtError(error: unknown) {
    void reportCrash(error, "render_error");
    // Preserve React's default developer-visible caught-error behavior.
    console.error(error);
  },
  onRecoverableError(error: unknown) {
    void reportCrash(error, "recoverable_error");
    reportToBrowser(error);
  },
};
