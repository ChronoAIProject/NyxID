import { getAssistantIdentityRevision, invalidateAccountEvidence, settleAccountEvidence } from "./assistant/identity";
import { useAuthStore } from "@/stores/auth-store";

const STORAGE_KEY = "nyxid:account-session";
const STALE_MS = 60_000;
const EVIDENCE_TIMEOUT_MS = 5_000;

/** Hints invalidate identity; only the cookie-authenticated profile is authority. */
export function installAccountSessionSync(): () => void {
  let channel: BroadcastChannel | undefined;
  let cached = false;
  let restoring = false;
  let disposed = false;
  let lastAttempt = Date.now();
  type Attempt = { revision: number; evidence: boolean; target?: string | null; controller: AbortController; timer?: ReturnType<typeof setTimeout> };
  let pending: Attempt | undefined;
  const seen = new Set<string>();
  const remember = (nonce: string) => {
    seen.add(nonce);
    if (seen.size > 100) seen.delete(seen.values().next().value!);
  };
  type SavedFocus = { element: HTMLElement; revision: number; range?: Range; start?: number | null; end?: number | null; direction?: "forward" | "backward" | "none" | null; moved: boolean };
  let focusBeforeHide: SavedFocus | undefined;
  let blurredFocus: SavedFocus | undefined;
  let blurTimer: ReturnType<typeof setTimeout> | undefined;
  const captureFocus = (element: HTMLElement): SavedFocus => {
    const selection = window.getSelection();
    return { element, revision: getAssistantIdentityRevision(), moved: false,
      ...(selection?.rangeCount ? { range: selection.getRangeAt(0).cloneRange() } : {}),
      ...(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement
        ? { start: element.selectionStart, end: element.selectionEnd, direction: element.selectionDirection } : {}),
    };
  };
  const focusin = (event: FocusEvent) => {
    blurredFocus = undefined;
    if (focusBeforeHide && event.target !== focusBeforeHide.element && event.target !== document.body)
      focusBeforeHide.moved = true;
  };
  // Chromium blurs the field before dispatching pagehide in the same task.
  // Keep that snapshot only until the next task, so an earlier deliberate blur
  // cannot cause a later navigation to restore an obsolete focus target.
  const focusout = (event: FocusEvent) => {
    if (event.relatedTarget || focusBeforeHide || !(event.target instanceof HTMLElement)) return;
    blurredFocus = captureFocus(event.target);
    clearTimeout(blurTimer);
    blurTimer = setTimeout(() => { blurredFocus = undefined; }, 0);
  };
  const reveal = () => {
    restoring = false;
    settleAccountEvidence();
    document.documentElement.removeAttribute("data-account-verifying");
    useAuthStore.setState({ isIdentityVerifying: false });
    const saved = focusBeforeHide;
    focusBeforeHide = undefined;
    if (saved && !saved.moved && saved.revision === getAssistantIdentityRevision() && saved.element.isConnected &&
        (!document.activeElement || document.activeElement === document.body || document.activeElement === saved.element)) {
      saved.element.focus({ preventScroll: true });
      if (saved.element instanceof HTMLInputElement || saved.element instanceof HTMLTextAreaElement) {
        if (saved.start != null && saved.end != null) saved.element.setSelectionRange(saved.start, saved.end, saved.direction ?? undefined);
      } else if (saved.range?.startContainer.isConnected && saved.range.endContainer.isConnected) {
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(saved.range);
      }
    }
  };
  const hide = () => {
    if (!document.documentElement.hasAttribute("data-account-verifying") && document.activeElement instanceof HTMLElement) {
      focusBeforeHide = document.activeElement === document.body && blurredFocus
        ? blurredFocus : captureFocus(document.activeElement);
    }
    document.documentElement.setAttribute("data-account-verifying", "");
    useAuthStore.setState({ isIdentityVerifying: true });
  };
  const verify = (evidence: boolean, target?: string | null) => {
    if (disposed) return;
    if (pending) {
      if (!evidence || (pending.evidence && pending.target === target)) return;
      // New evidence supersedes the read before it can commit a profile. This
      // includes an A hint while an A -> B check is still pending.
      clearTimeout(pending.timer);
      pending.controller.abort();
      pending = undefined;
    }
    lastAttempt = Date.now();
    const attempt: Attempt = { revision: getAssistantIdentityRevision(), evidence, target, controller: new AbortController() };
    pending = attempt;
    if (evidence) { hide(); armDeadline(attempt); }
    void useAuthStore.getState().verifySession({
      signal: attempt.controller.signal,
      isCurrent: () => !disposed && pending === attempt,
    }).then((confirmed) => {
      if (disposed || pending !== attempt) return;
      clearTimeout(attempt.timer);
      pending = undefined;
      if (attempt.evidence && !confirmed && attempt.revision === getAssistantIdentityRevision()) useAuthStore.getState().setUser(null);
      reveal();
    });
  };
  const armDeadline = (attempt: Attempt) => {
    if (attempt.timer) return;
    attempt.timer = setTimeout(() => {
      if (disposed || pending !== attempt) return;
      pending = undefined;
      attempt.controller.abort();
      if (attempt.revision === getAssistantIdentityRevision()) useAuthStore.getState().setUser(null);
      reveal();
    }, EVIDENCE_TIMEOUT_MS);
  };
  const receive = (value: unknown) => {
    if (cached || disposed) return;
    if (!value || typeof value !== "object" || !("userId" in value)) return;
    const id = value.userId;
    if (id !== null && typeof id !== "string") return;
    if ("nonce" in value && typeof value.nonce === "string") {
      if (seen.has(value.nonce)) return;
      remember(value.nonce);
    }
    if (!pending && id === (useAuthStore.getState().user?.id ?? null)) return;
    if (id !== (useAuthStore.getState().user?.id ?? null)) invalidateAccountEvidence();
    verify(true, id);
  };
  const openChannel = () => {
    if (channel) return;
    try {
      channel = new BroadcastChannel(STORAGE_KEY);
      channel.onmessage = (event) => receive(event.data);
    } catch { /* Storage fallback. */ }
  };
  openChannel();
  const storage = (event: StorageEvent) => {
    if (event.key !== STORAGE_KEY || !event.newValue) return;
    try {
      // Discard obsolete storage events queued while the document was cached.
      const latest = localStorage.getItem(STORAGE_KEY);
      if (restoring && latest && latest !== event.newValue) return;
      receive(JSON.parse(event.newValue));
    } catch { /* Malformed hint or unavailable storage. */ }
  };
  const pagehide = (event: PageTransitionEvent) => {
    if (event.persisted) {
      cached = true;
      channel?.close();
      channel = undefined;
      hide();
    } else reveal();
  };
  const pageshow = (event: PageTransitionEvent) => {
    if (event.persisted) {
      cached = false;
      restoring = true;
      openChannel();
      // The restore check covers the latest hint missed while cached. A new
      // nonce arriving during that check still schedules a fresh verification.
      try {
        const hint = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null");
        if (typeof hint?.nonce === "string") remember(hint.nonce);
      } catch { /* Storage unavailable; the authoritative check still runs. */ }
      invalidateAccountEvidence();
      verify(true);
    }
  };
  const visibility = () => {
    if (useAuthStore.getState().user && document.visibilityState === "visible" && Date.now() - lastAttempt >= STALE_MS)
      verify(false);
  };
  const timer = setInterval(visibility, STALE_MS);
  const stop = useAuthStore.subscribe((state, previous) => {
    if (state.user?.id === previous.user?.id) return;
    lastAttempt = Date.now();
    const hint = { userId: state.user?.id ?? null, nonce: crypto.randomUUID() };
    try { channel?.postMessage(hint); } catch { /* Storage fallback. */ }
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(hint)); } catch { /* Channel remains available. */ }
  });
  window.addEventListener("storage", storage);
  window.addEventListener("pageshow", pageshow);
  window.addEventListener("pagehide", pagehide);
  window.addEventListener("focus", visibility);
  document.addEventListener("focusin", focusin);
  document.addEventListener("focusout", focusout);
  document.addEventListener("visibilitychange", visibility);
  return () => {
    disposed = true;
    clearTimeout(pending?.timer);
    pending?.controller.abort();
    clearInterval(timer);
    clearTimeout(blurTimer);
    reveal();
    stop();
    channel?.close();
    window.removeEventListener("storage", storage);
    window.removeEventListener("pageshow", pageshow);
    window.removeEventListener("pagehide", pagehide);
    window.removeEventListener("focus", visibility);
    document.removeEventListener("focusin", focusin);
    document.removeEventListener("focusout", focusout);
    document.removeEventListener("visibilitychange", visibility);
  };
}
