import type { ApiErrorResponse } from "@/types/api";

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

  if (response.status === 401 && !NO_AUTH_STATE_CLEAR_ENDPOINTS.has(endpoint)) {
    const { useAuthStore } = await import("@/stores/auth-store");
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

  patch<T>(endpoint: string, body?: unknown): Promise<T> {
    return apiClient<T>(endpoint, { method: "PATCH", body });
  },

  delete<T>(endpoint: string): Promise<T> {
    return apiClient<T>(endpoint, { method: "DELETE" });
  },
} as const;
