import { useAuthStore } from "@/stores/auth-store";
import {
  beginIdentityVerification,
  finishIdentityVerification,
} from "./assistant/identity";

const STORAGE_KEY = "nyxid:account-session";
const CHANNEL = "nyxid:account-session";
const STALE_MS = 60_000;

/** Messages are invalidation hints only. The cookie-authenticated profile is authority. */
export function installAccountSessionSync(): () => void {
  let channel: BroadcastChannel | undefined;
  try {
    channel = new BroadcastChannel(CHANNEL);
  } catch {
    /* Storage event fallback. */
  }
  let pending: Promise<void> | undefined;
  let lastVerified = Date.now();
  let remote = false;
  let disposed = false;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let verifyAgain = false;
  const verify = () => {
    if (disposed) return;
    if (pending) {
      verifyAgain = true;
      return;
    }
    clearTimeout(retry);
    let confirmed = false;
    beginIdentityVerification();
    document.documentElement.setAttribute("data-account-verifying", "");
    useAuthStore.setState({ isIdentityVerifying: true });
    remote = true;
    pending = useAuthStore
      .getState()
      .checkAuth()
      .then((valid) => {
        confirmed = valid || useAuthStore.getState().user === null;
        if (confirmed) lastVerified = Date.now();
      })
      .finally(() => {
        remote = false;
        pending = undefined;
        if (disposed) return;
        if (!confirmed || verifyAgain) {
          verifyAgain = false;
          retry = setTimeout(verify, confirmed ? 0 : 5_000);
          return;
        }
        finishIdentityVerification();
        document.documentElement.removeAttribute("data-account-verifying");
        useAuthStore.setState({ isIdentityVerifying: false });
      });
  };
  const receive = (value: unknown) => {
    if (!value || typeof value !== "object" || !("userId" in value)) return;
    const id = value.userId;
    if (id !== null && typeof id !== "string") return;
    if (pending) {
      verifyAgain = true;
      return;
    }
    if (id === (useAuthStore.getState().user?.id ?? null)) return;
    verify();
  };
  if (channel) channel.onmessage = (event) => receive(event.data);
  const storage = (event: StorageEvent) => {
    if (event.key !== STORAGE_KEY || !event.newValue) return;
    try {
      receive(JSON.parse(event.newValue));
    } catch {
      /* Ignore malformed hints. */
    }
  };
  const pageshow = (event: PageTransitionEvent) => {
    if (event.persisted) verify();
  };
  const visibility = () => {
    if (
      document.visibilityState === "visible" &&
      Date.now() - lastVerified >= STALE_MS
    )
      verify();
  };
  const timer = setInterval(visibility, STALE_MS);
  const stop = useAuthStore.subscribe((state, previous) => {
    if (state.user !== previous.user) lastVerified = Date.now();
    if (state.user?.id === previous.user?.id) return;
    lastVerified = Date.now();
    if (remote) return;
    const hint = { userId: state.user?.id ?? null, nonce: crypto.randomUUID() };
    try {
      channel?.postMessage(hint);
    } catch {
      /* Storage event fallback. */
    }
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(hint));
    } catch {
      /* BroadcastChannel still works. */
    }
  });
  window.addEventListener("storage", storage);
  window.addEventListener("pageshow", pageshow);
  document.addEventListener("visibilitychange", visibility);
  return () => {
    disposed = true;
    clearTimeout(retry);
    clearInterval(timer);
    finishIdentityVerification();
    document.documentElement.removeAttribute("data-account-verifying");
    useAuthStore.setState({ isIdentityVerifying: false });
    stop();
    channel?.close();
    window.removeEventListener("storage", storage);
    window.removeEventListener("pageshow", pageshow);
    document.removeEventListener("visibilitychange", visibility);
  };
}
