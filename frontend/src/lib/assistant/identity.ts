import { toast } from "sonner";

type AssistantIdentityListener = (
  userId: string | null,
  previousUserId: string | null,
) => void;

let currentUserId: string | null = null;
let identityRevision = 0;
let requests = new AbortController();
const listeners = new Set<AssistantIdentityListener>();

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
  previousRequests.abort(new AccountChangedError());
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
      const controller = new AbortController();
      const abortEpoch = () => {
        controller.abort(epochSignal.reason);
        dispose();
      };
      const abortCaller = () => {
        controller.abort(signal.reason);
        dispose();
      };
      let disposed = false;
      dispose = () => {
        if (disposed) return;
        disposed = true;
        epochSignal.removeEventListener("abort", abortEpoch);
        signal.removeEventListener("abort", abortCaller);
      };
      epochSignal.addEventListener("abort", abortEpoch, { once: true });
      signal.addEventListener("abort", abortCaller, { once: true });
      if (epochSignal.aborted) abortEpoch();
      else if (signal.aborted) abortCaller();
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

// Raw streaming callers release their scope after EOF/cancellation. Native fetch
// owns body cancellation; no Response methods or streams are replaced.
const responseScopes = new WeakMap<Response, () => void>();
export function retainAccountResponse(response: Response, dispose: () => void): Response {
  if (response.body) responseScopes.set(response, dispose);
  else dispose();
  return response;
}
export function releaseAccountResponse(response: Response): void {
  responseScopes.get(response)?.();
  responseScopes.delete(response);
}
