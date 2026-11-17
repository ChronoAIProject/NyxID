import { toast } from "sonner";

type AssistantIdentityListener = (
  userId: string | null,
  previousUserId: string | null,
) => void;

let currentUserId: string | null = null;
let identityRevision = 0;
let requests = new AbortController();
const listeners = new Set<AssistantIdentityListener>();
// Neither the epoch nor a long-lived caller keeps fallback requests alive.
const fallbackChildren = new Set<WeakRef<AbortController>>();
type FallbackCleanup = { child: WeakRef<AbortController>; caller: WeakRef<AbortSignal>; listener: EventListener };
const fallbackCleanup = new Map<WeakRef<AbortController>, FallbackCleanup>();
function removeFallback(entry: FallbackCleanup): void {
  entry.caller.deref()?.removeEventListener("abort", entry.listener);
  fallbackChildren.delete(entry.child);
  fallbackCleanup.delete(entry.child);
  collectedFallbacks.unregister(entry.child);
}
const collectedFallbacks = new FinalizationRegistry<FallbackCleanup>(removeFallback);
// A separate closure scope ensures the caller listener captures only a WeakRef.
function forwardCallerAbort(child: WeakRef<AbortController>): EventListener {
  return (event) => child.deref()?.abort((event.target as AbortSignal).reason);
}

interface AccountRequest {
  readonly signal: AbortSignal;
  dispose(): void;
  assertCurrent(): void;
}

export function getAssistantIdentityUserId(): string | null {
  return currentUserId;
}

/** Changes even when an account switches away and back during one request. */
export function getAssistantIdentityRevision(): number {
  return identityRevision;
}

export function transitionAssistantIdentity(userId: string | null): void {
  if (userId === currentUserId) return;
  const previousUserId = currentUserId;
  currentUserId = userId;
  identityRevision += 1;
  const previousRequests = requests;
  requests = new AbortController();
  const children = [...fallbackChildren];
  fallbackChildren.clear();
  previousRequests.abort(new AccountChangedError());
  for (const child of children) {
    const controller = child.deref();
    if (controller) controller.abort(new AccountChangedError());
    else {
      const entry = fallbackCleanup.get(child);
      if (entry) removeFallback(entry);
    }
  }
  toast.dismiss();
  for (const listener of listeners) listener(userId, previousUserId);
}

export function subscribeAssistantIdentity(
  listener: AssistantIdentityListener,
): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Capture before any await, and check again after consuming response bodies. */
export function captureAccountRequest(signal?: AbortSignal): AccountRequest {
  const revision = identityRevision;
  const epochSignal = requests.signal;
  let dispose = () => {};
  let combined = epochSignal;
  if (signal && signal !== epochSignal) {
    if (typeof AbortSignal.any === "function") {
      combined = AbortSignal.any([epochSignal, signal]);
    } else {
      for (const child of fallbackChildren) {
        if (!child.deref()) removeFallback(fallbackCleanup.get(child)!);
      }
      const controller = new AbortController();
      const child = new WeakRef(controller);
      const listener = forwardCallerAbort(child);
      const entry = { child, caller: new WeakRef(signal), listener };
      fallbackChildren.add(child);
      fallbackCleanup.set(child, entry);
      collectedFallbacks.register(controller, entry, child);
      dispose = () => {
        removeFallback(entry);
        // This self-contained cycle keeps the controller alive while its signal
        // is in use, but is collectable with an abandoned request/response.
        controller.signal.removeEventListener("abort", dispose);
      };
      controller.signal.addEventListener("abort", dispose, { once: true });
      signal.addEventListener("abort", listener, { once: true });
      if (signal.aborted) controller.abort(signal.reason);
      combined = controller.signal;
    }
  }
  const assertCurrent = () => {
    if (revision !== identityRevision) throw new AccountChangedError();
    if (combined.aborted)
      throw (
        combined.reason ?? new DOMException("Request cancelled", "AbortError")
      );
  };
  return {
    signal: combined,
    dispose,
    assertCurrent,
  };
}

/** Only account changes are silent; caller/user cancellations keep their own handling. */
export class AccountChangedError extends DOMException {
  constructor() { super("Account changed", "AbortError"); }
}

export function isAccountAbort(error: unknown): error is AccountChangedError {
  return error instanceof AccountChangedError;
}
