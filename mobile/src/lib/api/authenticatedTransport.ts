import { decodeJwtSub } from "../auth/jwt";

export type RequestOptions = {
  expectedAccountId?: string;
  authorityGuard?: () => void;
  method?: "GET" | "POST" | "PATCH" | "PUT" | "DELETE";
  body?: unknown;
  rawBody?: ArrayBuffer;
  requiresAuth?: boolean;
  headers?: Record<string, string>;
  retryOnAuthFailure?: boolean;
  invalidateSessionOnAuthFailure?: boolean;
  signal?: AbortSignal;
};

type Session = { accessToken: string; userId?: string };
type HttpResponse = {
  ok: boolean;
  status: number;
  text: () => Promise<string>;
};

function identity(session: Session): string {
  return (
    session.userId ?? decodeJwtSub(session.accessToken) ?? session.accessToken
  );
}

function ensureNotAborted(signal?: AbortSignal) {
  if (signal?.aborted)
    throw Object.assign(new Error("Request cancelled"), { name: "AbortError" });
}

/** Share native JSON and streaming admission. Only an explicit HTTP 401 is
 * replayable: network/stream failures may already have caused an effect. */
export async function executeAuthenticatedRequest<
  R extends HttpResponse,
>(input: {
  url: string;
  options: RequestOptions;
  clientHeaders: Record<string, string>;
  transport: (
    url: string,
    init: {
      method: string;
      headers: Record<string, string>;
      body?: string | ArrayBuffer;
      signal?: AbortSignal;
    },
  ) => Promise<R>;
  loadSession: () => Promise<Session | null>;
  refresh: () => Promise<{ access_token: string } | null>;
  invalidate: () => Promise<void>;
  toError: (payload: unknown, status: number) => Error;
  getAuthEpoch?: () => number;
}): Promise<{ response: R; assertOwner: () => Promise<void> }> {
  const { options } = input;
  if (options.body !== undefined && options.rawBody !== undefined)
    throw new Error("Choose either JSON or raw request bytes.");
  const requiresAuth = options.requiresAuth ?? true;
  const originalEpoch = input.getAuthEpoch?.();
  ensureNotAborted(options.signal);
  const session = requiresAuth ? await input.loadSession() : null;
  if (requiresAuth && !session?.accessToken)
    throw new Error("auth_session_missing");
  const originalIdentity = session ? identity(session) : null;
  if (options.expectedAccountId && (!session || decodeJwtSub(session.accessToken) !== options.expectedAccountId))
    throw new Error("analytics_account_changed");
  const checkAdmission = () => {
    options.authorityGuard?.();
    ensureNotAborted(options.signal);
    if (requiresAuth && input.getAuthEpoch?.() !== originalEpoch) {
      throw Object.assign(new Error("auth_session_changed"), {
        name: "AbortError",
      });
    }
  };
  const checkOwner = async () => {
    checkAdmission();
    if (!requiresAuth) return;
    const current = await input.loadSession();
    checkAdmission();
    if (
      !current ||
      identity(current) !== originalIdentity ||
      input.getAuthEpoch?.() !== originalEpoch
    ) {
      throw Object.assign(new Error("auth_session_changed"), {
        name: "AbortError",
      });
    }
  };
  const headers = {
    Accept: "application/json",
    ...input.clientHeaders,
    ...options.headers,
    ...(options.body === undefined
      ? {}
      : { "Content-Type": "application/json" }),
    ...(session ? { Authorization: `Bearer ${session.accessToken}` } : {}),
  };
  const send = () => {
    // SecureStore and refresh can wait behind another native operation. A
    // cancelled action or account switch must never dispatch after that wait.
    checkAdmission();
    return input.transport(input.url, {
      method: options.method ?? "GET",
      headers,
      body:
        options.rawBody ??
        (options.body === undefined ? undefined : JSON.stringify(options.body)),
      signal: options.signal,
    });
  };
  await checkOwner();
  let response = await send();
  await checkOwner();
  if (
    response.status === 401 &&
    requiresAuth &&
    (options.retryOnAuthFailure ?? true)
  ) {
    const refreshed = await input.refresh();
    await checkOwner();
    if (refreshed) {
      headers.Authorization = `Bearer ${refreshed.access_token}`;
      response = await send();
      await checkOwner();
    }
  }
  if (!response.ok) {
    const raw = await response.text();
    await checkOwner();
    let payload: unknown = raw;
    try {
      payload = raw ? JSON.parse(raw) : null;
    } catch {
      /* Plain error bodies remain readable. */
    }
    if (
      response.status === 401 &&
      requiresAuth &&
      (options.invalidateSessionOnAuthFailure ?? true)
    ) {
      await input.invalidate();
    }
    throw input.toError(payload, response.status);
  }
  return { response, assertOwner: checkOwner };
}
