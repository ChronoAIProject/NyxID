import { getAssistantIdentityRevision } from "./assistant/identity";
import { useAuthStore } from "@/stores/auth-store";

const STORAGE_KEY = "nyxid:account-session";
const STALE_MS = 60_000;
const EVIDENCE_TIMEOUT_MS = 5_000;

/** Hints invalidate identity; only the cookie-authenticated profile is authority. */
export function installAccountSessionSync(): () => void {
  let channel: BroadcastChannel | undefined;
  try { channel = new BroadcastChannel(STORAGE_KEY); } catch { /* Storage fallback. */ }
  let disposed = false;
  let remote = false;
  let lastAttempt = Date.now();
  type Attempt = { revision: number; evidence: boolean; target?: string | null; again: boolean; timer?: ReturnType<typeof setTimeout> };
  let pending: Attempt | undefined;
  const seen = new Set<string>();
  const reveal = () => {
    document.documentElement.removeAttribute("data-account-verifying");
    useAuthStore.setState({ isIdentityVerifying: false });
  };
  const hide = () => {
    document.documentElement.setAttribute("data-account-verifying", "");
    useAuthStore.setState({ isIdentityVerifying: true });
  };
  const verify = (evidence: boolean, target?: string | null) => {
    if (disposed) return;
    if (pending) {
      if (evidence) {
        if (pending.evidence && pending.target !== target) pending.again = true;
        pending.target = target;
        pending.evidence = true;
        hide();
        armDeadline(pending);
      }
      return;
    }
    lastAttempt = Date.now();
    const attempt: Attempt = { revision: getAssistantIdentityRevision(), evidence, target, again: false };
    pending = attempt;
    if (evidence) { hide(); armDeadline(attempt); }
    remote = true;
    void useAuthStore.getState().verifySession().then((confirmed) => {
      if (disposed || pending !== attempt) return;
      clearTimeout(attempt.timer);
      pending = undefined;
      if (attempt.evidence && !confirmed && attempt.revision === getAssistantIdentityRevision()) useAuthStore.getState().setUser(null);
      if (attempt.again) verify(true, attempt.target);
      else reveal();
    }).finally(() => { if (!pending) remote = false; });
  };
  const armDeadline = (attempt: Attempt) => {
    if (attempt.timer) return;
    attempt.timer = setTimeout(() => {
      if (disposed || pending !== attempt) return;
      pending = undefined;
      if (attempt.revision === getAssistantIdentityRevision()) useAuthStore.getState().setUser(null);
      remote = false;
      reveal();
    }, EVIDENCE_TIMEOUT_MS);
  };
  const receive = (value: unknown) => {
    if (!value || typeof value !== "object" || !("userId" in value)) return;
    const id = value.userId;
    if (id !== null && typeof id !== "string") return;
    if ("nonce" in value && typeof value.nonce === "string") {
      if (seen.has(value.nonce)) return;
      seen.add(value.nonce);
      if (seen.size > 100) seen.delete(seen.values().next().value!);
    }
    if (id === (useAuthStore.getState().user?.id ?? null)) return;
    verify(true, id);
  };
  if (channel) channel.onmessage = (event) => receive(event.data);
  const storage = (event: StorageEvent) => {
    if (event.key !== STORAGE_KEY || !event.newValue) return;
    try { receive(JSON.parse(event.newValue)); } catch { /* Malformed hint. */ }
  };
  const pageshow = (event: PageTransitionEvent) => { if (event.persisted) verify(true); };
  const visibility = () => {
    if (useAuthStore.getState().user && document.visibilityState === "visible" && Date.now() - lastAttempt >= STALE_MS)
      verify(false);
  };
  const timer = setInterval(visibility, STALE_MS);
  const stop = useAuthStore.subscribe((state, previous) => {
    if (state.user?.id === previous.user?.id) return;
    lastAttempt = Date.now();
    if (remote) return;
    const hint = { userId: state.user?.id ?? null, nonce: crypto.randomUUID() };
    try { channel?.postMessage(hint); } catch { /* Storage fallback. */ }
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(hint)); } catch { /* Channel remains available. */ }
  });
  window.addEventListener("storage", storage);
  window.addEventListener("pageshow", pageshow);
  document.addEventListener("visibilitychange", visibility);
  return () => {
    disposed = true;
    clearTimeout(pending?.timer);
    clearInterval(timer);
    reveal();
    stop();
    channel?.close();
    window.removeEventListener("storage", storage);
    window.removeEventListener("pageshow", pageshow);
    document.removeEventListener("visibilitychange", visibility);
  };
}
