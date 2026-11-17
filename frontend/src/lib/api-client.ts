import {
  accountResponse,
  captureAccountRequest,
  getAssistantIdentityRevision,
} from "./assistant/identity";
import {
  ApiError,
  apiErrorBody,
  responseApiError,
  networkApiError,
} from "./api-error";
import type { ApiErrorResponse } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import { isTelemetryActive } from "@/lib/telemetry";
import {
  currentCreditsActor,
  reportCreditsDenialHttp,
  type CreditsDenialRequest,
} from "@/lib/credits-denial";

export function apiUrl(endpoint: string, apiBaseUrl = ""): string {
  return `${apiBaseUrl.replace(/\/+$/, "")}/api/v1${endpoint}`;
}

export { ApiError } from "./api-error";

interface RequestOptions {
  /** Internal scope shared through response body consumption. */
  readonly accountRequest?: ReturnType<typeof captureAccountRequest>;
  /** CMA adapters alone opt into CMA problem/Responses error documents. */
  readonly errorFormat?: "cma";
  /** Opt-in fence for requests bound to a captured account or local CLI session. */
  readonly authorityGuard?: () => void;
  readonly apiBaseUrl?: string;
  readonly credentials?: RequestCredentials;
  readonly method?: string;
  readonly body?: unknown;
  readonly headers?: Record<string, string>;
  readonly signal?: AbortSignal;
  readonly onResponse?: (response: Response) => void;
  /**
   * Opt this request out of the global sign-out on 401. For page-scoped
   * integrations (e.g. the /assistant transports calling a downstream through
   * the proxy) a 401 means that integration rejected the request — it says
   * nothing about the NyxID session, so it must surface as a page-level error
   * instead of clearing auth state and bouncing the user to /login. Requests
   * that authenticate against NyxID itself must NOT set this.
   */
  readonly preserveSessionOn401?: boolean;
  /**
   * Foreground operations only: an HTTP 402 `insufficient_credits` opens the
   * out-of-credits dialog for this operation key. The `ApiError` is still
   * thrown so callers keep their inline error handling.
   */
  readonly creditsDenial?: CreditsDenialRequest;
}

// Endpoints that should not clear the global auth state on 401 because they
// are part of the auth flow itself.
const NO_AUTH_STATE_CLEAR_ENDPOINTS = new Set([
  "/auth/login",
  "/auth/register",
  "/auth/refresh",
  "/auth/forgot-password",
  "/auth/reset-password",
  "/auth/verify-email",
  "/auth/setup",
  "/auth/mfa/verify",
  "/auth/mfa/cancel",
]);

let sessionConfirmation: {
  revision: number;
  promise: Promise<boolean>;
} | null = null;
let lastSessionConfirmation: {
  at: number;
  valid: boolean;
  user: unknown;
} | null = null;

async function confirmSessionStillValid(): Promise<boolean> {
  const now = Date.now();
  const user = useAuthStore.getState().user;
  if (
    lastSessionConfirmation &&
    lastSessionConfirmation.user === user &&
    now - lastSessionConfirmation.at < 500
  ) {
    return lastSessionConfirmation.valid;
  }
  const revision = getAssistantIdentityRevision();
  if (!sessionConfirmation || sessionConfirmation.revision !== revision) {
    const request = captureAccountRequest();
    const promise = fetch(apiUrl("/users/me"), {
      ...buildFetchConfig({ preserveSessionOn401: true }),
      method: "GET",
      signal: request.signal,
    })
      // Only a confirmed identity 401 proves that the NyxID session is gone.
      // A 2xx, another HTTP status, or a transient network failure must keep
      // the browser session intact.
      .then(
        (response) =>
          response.status !== 401 || useAuthStore.getState().user !== user,
      )
      .catch(() => true)
      .then((valid) => {
        lastSessionConfirmation = { at: Date.now(), valid, user };
        return valid;
      })
      .finally(() => {
        request.dispose();
        if (sessionConfirmation?.revision === revision)
          sessionConfirmation = null;
      });
    sessionConfirmation = { revision, promise };
  }
  return sessionConfirmation.promise;
}

export function buildFetchConfig(options: RequestOptions): RequestInit {
  const { method = "GET", body, headers = {}, signal } = options;

  // This device header only narrows the persisted account permission. The
  // backend rechecks it and consent revisions at admission and dispatch.
  const telemetryHeaders: Record<string, string> = isTelemetryActive()
    ? {
        "X-NyxID-Client": "ui",
        "X-NyxID-Analytics": "enabled",
        "X-NyxID-Analytics-Account": useAuthStore.getState().user?.id ?? "",
      }
    : {};

  const config: RequestInit = {
    method,
    headers: {
      "Content-Type": "application/json",
      ...telemetryHeaders,
      ...headers,
    },
    credentials: options.credentials ?? "include",
    signal,
  };

  if (body !== undefined) {
    config.body = JSON.stringify(body);
  }

  return config;
}

function redirectToConsentIfRequired(error: ApiErrorResponse): void {
  if (error.error !== "consent_required" || !error.consent_url) {
    return;
  }

  if (typeof window !== "undefined") {
    const url = error.consent_url;
    const request = captureAccountRequest();
    void import("./navigation").then(({ openExternal }) => {
      try {
        request.assertCurrent();
        openExternal(url);
      } catch {
        /* Identity changed. */
      }
    });
  }
}

export async function apiClient<T>(
  endpoint: string,
  options: RequestOptions = {},
): Promise<T> {
  const request =
    options.accountRequest ?? captureAccountRequest(options.signal);
  const guard = options.authorityGuard;
  options = {
    ...options,
    accountRequest: request,
    signal: request.signal,
    authorityGuard: () => {
      request.assertCurrent();
      guard?.();
    },
  };
  try {
    if (endpoint !== "/users/me") await request.ready();
    if (import.meta.env.DEV) {
      const { isMockMode, getMockResponse } = await import("./mock-data");
      options.authorityGuard?.();
      if (isMockMode()) {
        const mock = getMockResponse(
          endpoint,
          options.method ?? "GET",
          options.body,
        );
        if (mock !== undefined) return mock as T;
      }
    }

    const response = await apiFetch(endpoint, options);
    options.authorityGuard?.();
    if (response.status === 204) return undefined as T;
    const body = (await response.json().catch(async () => {
      if (endpoint !== "/users/me") await request.ready();
      options.authorityGuard?.();
      throw new ApiError(
        response.status,
        apiErrorBody(null, response.status),
        response.headers,
      );
    })) as T;
    if (endpoint !== "/users/me") await request.ready();
    options.authorityGuard?.();
    return body;
  } finally {
    request.dispose();
  }
}

/** Authenticated transport shared by JSON and streaming API requests. */
export async function apiFetch(
  endpoint: string,
  options: RequestOptions = {},
): Promise<Response> {
  const ownsRequest = !options.accountRequest;
  const request =
    options.accountRequest ?? captureAccountRequest(options.signal);
  const guard = options.authorityGuard;
  options = {
    ...options,
    accountRequest: request,
    signal: request.signal,
    authorityGuard: () => {
      request.assertCurrent();
      guard?.();
    },
  };
  try {
    if (endpoint !== "/users/me") await request.ready();
    const userAtRequest = useAuthStore.getState().user;
    const actorId = options.creditsDenial ? currentCreditsActor() : null;
    options.authorityGuard?.();
    const response = await fetch(
      apiUrl(endpoint, options.apiBaseUrl),
      buildFetchConfig(options),
    ).catch((cause: unknown) => {
      request.assertCurrent();
      throw networkApiError(cause);
    });
    if (endpoint !== "/users/me") await request.ready();
    // A late response belongs to its original authority, including any 401.
    options.authorityGuard?.();
    try {
      options.onResponse?.(response);
    } catch {
      // Response observers are diagnostic-only and must not affect the request.
    }

    if (!response.ok) {
      const error = await responseApiError(response, options.errorFormat);
      if (endpoint !== "/users/me") await request.ready();
      options.authorityGuard?.();
      if (
        response.status === 401 &&
        !options.preserveSessionOn401 &&
        !NO_AUTH_STATE_CLEAR_ENDPOINTS.has(endpoint)
      ) {
        if (endpoint === "/users/me") {
          useAuthStore.getState().setUser(null);
        } else {
          const sessionStillValid = await confirmSessionStillValid();
          options.authorityGuard?.();
          if (
            !sessionStillValid &&
            useAuthStore.getState().user === userAtRequest
          ) {
            useAuthStore.getState().setUser(null);
          }
        }
      } else {
        redirectToConsentIfRequired(error.errorResponse);
        reportCreditsDenialHttp(
          options.creditsDenial,
          response.status,
          error.errorResponse,
          actorId,
        );
      }
      throw error;
    }

    return ownsRequest ? accountResponse(response, request) : response;
  } catch (error) {
    request.dispose();
    throw error;
  }
}

/** Authenticated binary response helper used by attachment-style downloads. */
export async function apiBlobClient(
  endpoint: string,
  options: RequestOptions = {},
): Promise<{ blob: Blob; filename?: string }> {
  const request =
    options.accountRequest ?? captureAccountRequest(options.signal);
  try {
    const response = await apiFetch(endpoint, {
      ...options,
      accountRequest: request,
      signal: request.signal,
    });
    const disposition = response.headers.get("Content-Disposition") ?? "";
    const match = disposition.match(/filename="([^"]+)"/i);
    const blob = await response.blob();
    await request.ready();
    return { blob, filename: match?.[1] };
  } catch (error) {
    request.assertCurrent();
    throw error;
  } finally {
    request.dispose();
  }
}

export const api = {
  get<T>(
    endpoint: string,
    options?: Pick<RequestOptions, "authorityGuard">,
  ): Promise<T> {
    return apiClient<T>(endpoint, options);
  },

  post<T>(endpoint: string, body?: unknown): Promise<T> {
    return apiClient<T>(endpoint, { method: "POST", body });
  },

  put<T>(
    endpoint: string,
    body?: unknown,
    options?: Pick<RequestOptions, "authorityGuard">,
  ): Promise<T> {
    return apiClient<T>(endpoint, { ...options, method: "PUT", body });
  },

  patch<T>(
    endpoint: string,
    body?: unknown,
    options?: { signal?: AbortSignal },
  ): Promise<T> {
    return apiClient<T>(endpoint, {
      method: "PATCH",
      body,
      signal: options?.signal,
    });
  },

  delete<T>(endpoint: string): Promise<T> {
    return apiClient<T>(endpoint, { method: "DELETE" });
  },
} as const;
