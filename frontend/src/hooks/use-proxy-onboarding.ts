import { useEffect, useState } from "react";

/**
 * localStorage key for the "make first proxy call" activation step.
 * Mirrors the brief's `nyxid.first_proxy_call_succeeded`. Set once the
 * user runs the verify-key card from the api-key detail page and the
 * brokered proxy test returns a green 200 / red 4xx pair.
 *
 * Kept separate from the existing per-user onboarding takeover state in
 * `hooks/use-onboarding.ts` (which gates a first-run wizard over the
 * dashboard chrome). This key is a lightweight client-side flag that
 * surfaces the activation aha on the dashboard checklist — it does not
 * ride through `/users/me` or block any route.
 */
export const FIRST_PROXY_CALL_SUCCEEDED_KEY = "nyxid.first_proxy_call_succeeded";

/**
 * Custom event dispatched by the VerifyKeyCard after a successful brokered
 * proxy test (200 allowed + 4xx denied). The dashboard listens for it and
 * flips the onboarding checklist's "make first proxy call" step to done.
 */
export const FIRST_PROXY_CALL_SUCCEEDED_EVENT =
  "nyxid:first-proxy-call-succeeded";

/**
 * Lifecycle events dispatched by the VerifyKeyCard while the brokered test
 * is in flight. The dashboard's checklist CTA shows a spinner-disabled
 * state while loading so the user sees "test running" instead of a stale
 * CTA when they return.
 */
export const VERIFY_KEY_LOADING_START_EVENT = "nyxid:verify-key-loading-start";
export const VERIFY_KEY_LOADING_END_EVENT = "nyxid:verify-key-loading-end";

function readStoredFlag(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return localStorage.getItem(FIRST_PROXY_CALL_SUCCEEDED_KEY) === "1";
  } catch {
    return false;
  }
}

function writeStoredFlag(value: boolean): void {
  if (typeof window === "undefined") return;
  try {
    localStorage.setItem(FIRST_PROXY_CALL_SUCCEEDED_KEY, value ? "1" : "0");
  } catch {
    // localStorage may be blocked (private mode / storage quota). Swallow —
    // the in-memory state still flips via the event listener.
  }
}

/**
 * Tracks whether the user has ever completed the "make first proxy call"
 * activation step. Persists to localStorage so the flag survives reloads.
 * Also surfaces a transient `verifyKeyLoading` flag — driven by custom
 * events emitted from the VerifyKeyCard — so the checklist's primary CTA
 * can show a spinner while the in-flight test runs on another route.
 */
export function useProxyOnboarding() {
  const [firstProxyCallSucceeded, setFirstProxyCallSucceeded] = useState<boolean>(
    readStoredFlag,
  );
  const [verifyKeyLoading, setVerifyKeyLoading] = useState(false);

  useEffect(() => {
    function onSucceeded() {
      writeStoredFlag(true);
      setFirstProxyCallSucceeded(true);
    }
    function onLoadingStart() {
      setVerifyKeyLoading(true);
    }
    function onLoadingEnd() {
      setVerifyKeyLoading(false);
    }
    window.addEventListener(
      FIRST_PROXY_CALL_SUCCEEDED_EVENT,
      onSucceeded,
    );
    window.addEventListener(VERIFY_KEY_LOADING_START_EVENT, onLoadingStart);
    window.addEventListener(VERIFY_KEY_LOADING_END_EVENT, onLoadingEnd);
    return () => {
      window.removeEventListener(
        FIRST_PROXY_CALL_SUCCEEDED_EVENT,
        onSucceeded,
      );
      window.removeEventListener(VERIFY_KEY_LOADING_START_EVENT, onLoadingStart);
      window.removeEventListener(VERIFY_KEY_LOADING_END_EVENT, onLoadingEnd);
    };
  }, []);

  return { firstProxyCallSucceeded, verifyKeyLoading } as const;
}
