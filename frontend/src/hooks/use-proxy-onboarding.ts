import { useSyncExternalStore } from "react";
import { useAuthStore } from "@/stores/auth-store";

/** Prefix only. Never adopt the legacy unowned value at this exact key. */
export const FIRST_PROXY_CALL_SUCCEEDED_KEY =
  "nyxid.first_proxy_call_succeeded";
export const FIRST_PROXY_CALL_SUCCEEDED_EVENT =
  "nyxid:first-proxy-call-succeeded";
export const VERIFY_KEY_LOADING_START_EVENT = "nyxid:verify-key-loading-start";
export const VERIFY_KEY_LOADING_END_EVENT = "nyxid:verify-key-loading-end";

const sessionSuccesses = new Set<string>();
const pendingProbes = new Map<string, number>();

function storageKey(userId: string): string {
  return `${FIRST_PROXY_CALL_SUCCEEDED_KEY}:${encodeURIComponent(userId)}`;
}

function readStoredFlag(userId: string | null): boolean {
  if (!userId || typeof window === "undefined") return false;
  try {
    return (
      localStorage.getItem(storageKey(userId)) === "1" ||
      sessionSuccesses.has(userId)
    );
  } catch {
    return sessionSuccesses.has(userId);
  }
}

/** Call at completion with the identity captured BEFORE starting the request. */
export function recordFirstProxyCallSucceeded(userId: string | null): void {
  if (!userId || typeof window === "undefined") return;
  try {
    localStorage.setItem(storageKey(userId), "1");
  } catch {
    // Storage can be blocked; retain a session-only result for this owner.
    sessionSuccesses.add(userId);
  }
  window.dispatchEvent(
    new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT, {
      detail: { userId },
    }),
  );
}

/** Loading outlives route unmounts; concurrent probes finish independently. */
export function beginProxyVerification(userId: string | null): () => void {
  if (!userId || typeof window === "undefined") return () => {};
  pendingProbes.set(userId, (pendingProbes.get(userId) ?? 0) + 1);
  window.dispatchEvent(
    new CustomEvent(VERIFY_KEY_LOADING_START_EVENT, {
      detail: { userId },
    }),
  );
  let finished = false;
  return () => {
    if (finished) return;
    finished = true;
    const remaining = (pendingProbes.get(userId) ?? 1) - 1;
    if (remaining > 0) pendingProbes.set(userId, remaining);
    else pendingProbes.delete(userId);
    window.dispatchEvent(
      new CustomEvent(VERIFY_KEY_LOADING_END_EVENT, {
        detail: { userId },
      }),
    );
  };
}

function subscribe(onChange: () => void): () => void {
  const events = [
    FIRST_PROXY_CALL_SUCCEEDED_EVENT,
    VERIFY_KEY_LOADING_START_EVENT,
    VERIFY_KEY_LOADING_END_EVENT,
    "storage",
  ];
  for (const event of events) window.addEventListener(event, onChange);
  return () => {
    for (const event of events) window.removeEventListener(event, onChange);
  };
}

/** Dashboard consumes persisted results; it does not own their recording. */
export function useProxyOnboarding() {
  const userId = useAuthStore((s) =>
    s.isAuthenticated ? (s.user?.id ?? null) : null,
  );
  const firstProxyCallSucceeded = useSyncExternalStore(
    subscribe,
    () => readStoredFlag(userId),
    () => false,
  );
  const verifyKeyLoading = useSyncExternalStore(
    subscribe,
    () => userId !== null && (pendingProbes.get(userId) ?? 0) > 0,
    () => false,
  );
  return { firstProxyCallSucceeded, verifyKeyLoading } as const;
}
