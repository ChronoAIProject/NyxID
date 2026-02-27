import type { ApiErrorResponse } from "@/types/api";
import { API_BASE } from "./urls";

const BASE_URL = API_BASE;

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
}

// Token refresh lock: when a 401 triggers a refresh, all concurrent
// requests wait on the same promise instead of each firing their own
// refresh call. Resets to null after the refresh settles.
let refreshPromise: Promise<boolean> | null = null;

// Endpoints that should never trigger a token refresh (they are part of
// the auth flow itself and would cause infinite loops).
const NO_REFRESH_ENDPOINTS = new Set([
  "/auth/login",
  "/auth/register",
  "/auth/refresh",
  "/auth/forgot-password",
  "/auth/reset-password",
  "/auth/verify-email",
  "/auth/setup",
]);

async function attemptTokenRefresh(): Promise<boolean> {
  try {
    const response = await fetch(`${BASE_URL}/auth/refresh`, {
      method: "POST",
      credentials: "include",
      headers: { "Content-Type": "application/json" },
    });
    return response.ok;
  } catch {
    return false;
  }
}

function buildFetchConfig(options: RequestOptions): RequestInit {
  const { method = "GET", body, headers = {} } = options;

  const config: RequestInit = {
    method,
    headers: {
      "Content-Type": "application/json",
      ...headers,
    },
    credentials: "include",
  };

  if (body !== undefined) {
    config.body = JSON.stringify(body);
  }

  return config;
}

async function parseErrorResponse(response: Response): Promise<ApiErrorResponse> {
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
  const config = buildFetchConfig(options);
  const url = `${BASE_URL}${endpoint}`;

  const response = await fetch(url, config);

  // On 401, attempt a single token refresh then retry the original request.
  // Auth endpoints are excluded to avoid infinite loops.
  if (response.status === 401 && !NO_REFRESH_ENDPOINTS.has(endpoint)) {
    // Coalesce concurrent refresh attempts behind a single promise
    if (refreshPromise === null) {
      refreshPromise = attemptTokenRefresh().finally(() => {
        refreshPromise = null;
      });
    }

    const refreshed = await refreshPromise;

    if (refreshed) {
      // Retry the original request with the new access token cookie
      const retryResponse = await fetch(url, buildFetchConfig(options));

      if (!retryResponse.ok) {
        const errorBody = await parseErrorResponse(retryResponse);
        redirectToConsentIfRequired(errorBody);
        throw new ApiError(retryResponse.status, errorBody);
      }

      if (retryResponse.status === 204) {
        return undefined as T;
      }

      return retryResponse.json() as Promise<T>;
    }

    // Refresh failed -- throw the original 401
    throw new ApiError(401, await parseErrorResponse(response));
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

  patch<T>(endpoint: string, body?: unknown): Promise<T> {
    return apiClient<T>(endpoint, { method: "PATCH", body });
  },

  delete<T>(endpoint: string): Promise<T> {
    return apiClient<T>(endpoint, { method: "DELETE" });
  },
} as const;
