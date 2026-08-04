import type { ApiErrorResponse } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import { isTelemetryActive } from "@/lib/telemetry";

const API_ORIGIN = "";

const BASE_URL = `${API_ORIGIN}/api/v1`;

export class ApiError extends Error {
  readonly status: number;
  readonly errorCode: number;
  readonly errorResponse: ApiErrorResponse;

  constructor(status: number, response: ApiErrorResponse) {
    super(response.message);
    this.name = "ApiError";
    this.status = status;
    this.errorCode = response.error_code;
    this.errorResponse = response;
  }
}

interface RequestOptions {
  readonly method?: string;
  readonly body?: unknown;
  readonly headers?: Record<string, string>;
  readonly signal?: AbortSignal;
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
]);

function buildFetchConfig(options: RequestOptions): RequestInit {
  const { method = "GET", body, headers = {}, signal } = options;

  // Surface identification for server-side telemetry. Only attached
  // once the runtime telemetry client has been constructed on this
  // page load. Keying off the live `isTelemetryActive()` (not just
  // persisted consent) means a browser with stale consent from an
  // earlier telemetry-on deploy will NOT leak these headers after
  // the operator turns telemetry off at the backend — the PostHog
  // client never initialized this session, so the surface header is
  // pointless anyway.
  const telemetryHeaders: Record<string, string> = isTelemetryActive()
    ? { "X-NyxID-Client": "ui" }
    : {};

  const config: RequestInit = {
    method,
    headers: {
      "Content-Type": "application/json",
      ...telemetryHeaders,
      ...headers,
    },
    credentials: "include",
    signal,
  };

  if (body !== undefined) {
    config.body = JSON.stringify(body);
  }

  return config;
}

async function parseErrorResponse(
  response: Response,
): Promise<ApiErrorResponse> {
  try {
    return (await response.json()) as ApiErrorResponse;
  } catch {
    return {
      error: "unknown_error",
      error_code: -1,
      message: `Request failed with status ${String(response.status)}`,
    };
  }
}

function redirectToConsentIfRequired(error: ApiErrorResponse): void {
  if (error.error !== "consent_required" || !error.consent_url) {
    return;
  }

  if (typeof window !== "undefined") {
    const url = error.consent_url;
    void import("./navigation").then(({ openExternal }) => openExternal(url));
  }
}

export async function apiClient<T>(
  endpoint: string,
  options: RequestOptions = {},
): Promise<T> {
  if (import.meta.env.DEV) {
    const { isMockMode, getMockResponse } = await import("./mock-data");
    if (isMockMode()) {
      const mock = getMockResponse(endpoint);
      if (mock !== undefined) return mock as T;
    }
  }

  const config = buildFetchConfig(options);
  const url = `${BASE_URL}${endpoint}`;

  const response = await fetch(url, config);

  if (response.status === 401 && !NO_AUTH_STATE_CLEAR_ENDPOINTS.has(endpoint)) {
    useAuthStore.getState().setUser(null);
  }

  if (!response.ok) {
    const errorBody = await parseErrorResponse(response);
    redirectToConsentIfRequired(errorBody);
    throw new ApiError(response.status, errorBody);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return response.json() as Promise<T>;
}

export const api = {
  get<T>(endpoint: string): Promise<T> {
    return apiClient<T>(endpoint);
  },

  post<T>(endpoint: string, body?: unknown): Promise<T> {
    return apiClient<T>(endpoint, { method: "POST", body });
  },

  put<T>(endpoint: string, body?: unknown): Promise<T> {
    return apiClient<T>(endpoint, { method: "PUT", body });
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
