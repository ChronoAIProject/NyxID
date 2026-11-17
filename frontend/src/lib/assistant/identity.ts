type AssistantIdentityListener = (
  userId: string | null,
  previousUserId: string | null,
) => void;

let currentUserId: string | null = null;
let identityRevision = 0;
let requests = new AbortController();
let verification: { promise: Promise<void>; resolve: () => void } | undefined;
const listeners = new Set<AssistantIdentityListener>();

interface AccountRequest {
  readonly signal: AbortSignal;
  dispose(): void;
  assertCurrent(): void;
  ready(): Promise<void>;
  fork(): AccountRequest;
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
  previousRequests.abort(new DOMException("Account changed", "AbortError"));
  for (const listener of listeners) listener(userId, previousUserId);
}

export function subscribeAssistantIdentity(
  listener: AssistantIdentityListener,
): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Hold ordinary requests while a shared browser cookie is being verified. */
export function beginIdentityVerification(): void {
  if (verification) return;
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  verification = { promise, resolve };
}

export function finishIdentityVerification(): void {
  const pending = verification;
  verification = undefined;
  pending?.resolve();
}

export function isIdentityVerificationPending(): boolean {
  return verification !== undefined;
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
    if (combined.aborted)
      throw (
        combined.reason ?? new DOMException("Request cancelled", "AbortError")
      );
    if (revision !== identityRevision)
      throw new DOMException(
        "Account changed or request cancelled",
        "AbortError",
      );
  };
  return {
    signal: combined,
    dispose,
    assertCurrent,
    fork(): AccountRequest {
      assertCurrent();
      return captureAccountRequest(signal);
    },
    async ready() {
      while (verification) {
        assertCurrent();
        const pending = verification.promise;
        await new Promise<void>((resolve, reject) => {
          const abort = () => {
            cleanup();
            reject(combined.reason);
          };
          const cleanup = () => combined.removeEventListener("abort", abort);
          combined.addEventListener("abort", abort, { once: true });
          void pending.then(() => {
            cleanup();
            resolve();
          });
          if (combined.aborted) abort();
        });
      }
      assertCurrent();
    },
  };
}

/** Keep streaming scopes alive until consumption/cancellation, then detach fallback listeners. */
export function accountResponse(
  response: Response,
  request: ReturnType<typeof captureAccountRequest>,
): Response {
  // Wrap lazily so diagnostics can clone the original response, and unconsumed
  // fixture responses retain ordinary Response semantics.
  if (!response.body) request.dispose();
  let wrapped: Response | undefined;
  const consume = () => {
    if (wrapped) return wrapped;
    if (!response.body) {
      request.dispose();
      return response;
    }
    const reader = response.body.getReader();
    let abort = () => {};
    const cleanup = () => {
      request.signal.removeEventListener("abort", abort);
      request.dispose();
    };
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        abort = () => {
          controller.error(
            request.signal.reason ??
              new DOMException("Account changed", "AbortError"),
          );
          void reader.cancel().catch(() => undefined);
          cleanup();
        };
        request.signal.addEventListener("abort", abort, { once: true });
        if (request.signal.aborted) abort();
      },
      async pull(controller) {
        try {
          const chunk = await reader.read();
          await request.ready();
          if (chunk.done) {
            controller.close();
            cleanup();
          } else controller.enqueue(chunk.value);
        } catch (error) {
          controller.error(error);
          cleanup();
        }
      },
      async cancel(reason) {
        cleanup();
        await reader.cancel(reason);
      },
    });
    wrapped = new Response(body, {
      status: response.status,
      statusText: response.statusText,
      headers: response.headers,
    });
    return wrapped;
  };
  return new Proxy(response, {
    get(target, key) {
      if (key === "body") return consume().body;
      if (key === "bodyUsed") return wrapped?.bodyUsed ?? response.bodyUsed;
      if (key === "clone")
        return () => {
          request.assertCurrent();
          return accountResponse(consume().clone(), request.fork());
        };
      if (
        ["json", "text", "blob", "arrayBuffer", "formData", "bytes"].includes(
          String(key),
        )
      ) {
        return async (...args: unknown[]) => {
          await request.ready();
          const result: unknown = await Reflect.apply(
            Reflect.get(consume(), key),
            consume(),
            args,
          );
          await request.ready();
          return result;
        };
      }
      const value = Reflect.get(target, key, target) as unknown;
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
}

/** Account cancellation is intentionally silent in page error handlers. */
export function isAccountAbort(error: unknown): boolean {
  return (
    !!error &&
    typeof error === "object" &&
    "name" in error &&
    error.name === "AbortError"
  );
}
