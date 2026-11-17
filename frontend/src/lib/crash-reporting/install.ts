import { reportCrash } from "./reporter";
import type { ErrorInfo } from "react";
/** Listeners observe without suppressing browser or React fallback behavior. */
export function installCrashReporting(): () => void {
  const error = (event: ErrorEvent | Event) => {
    if (event instanceof ErrorEvent) void reportCrash(event.error, "js_error");
    else {
      const target = event.target;
      const asset = target instanceof HTMLScriptElement ? target.src :
        target instanceof HTMLLinkElement && (target.relList.contains("stylesheet") || target.relList.contains("modulepreload")) ? target.href : null;
      if (!asset) return;
      try {
        const url = new URL(asset, window.location.origin);
        if (url.origin === window.location.origin && url.pathname.startsWith("/assets/"))
          void reportCrash(undefined, "resource_error");
      } catch { /* Invalid asset locations are outside capture scope. */ }
    }
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
  onUncaughtError(error: unknown, errorInfo?: ErrorInfo) {
    void reportCrash(error, "js_error");
    if (import.meta.env.DEV && errorInfo?.componentStack)
      console.error(error, errorInfo.componentStack);
    reportToBrowser(error);
  },
  onCaughtError(error: unknown, errorInfo?: ErrorInfo) {
    void reportCrash(error, "render_error");
    // Preserve React's default developer-visible caught-error behavior.
    if (import.meta.env.DEV && errorInfo?.componentStack)
      console.error(error, errorInfo.componentStack);
    else console.error(error);
  },
  onRecoverableError(error: unknown, errorInfo?: ErrorInfo) {
    void reportCrash(error, "recoverable_error");
    if (import.meta.env.DEV && errorInfo?.componentStack)
      console.error(error, errorInfo.componentStack);
    reportToBrowser(error);
  },
};
