import type { ApiErrorResponse } from "@/types/api";

const BASE_URL = "/api/v1";

export class ApiError extends Error {
  readonly status: number;
  readonly errorCode: string;
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

export async function apiClient<T>(
  endpoint: string,
  options: RequestOptions = {},
): Promise<T> {
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

  const response = await fetch(`${BASE_URL}${endpoint}`, config);

  if (!response.ok) {
    let errorResponse: ApiErrorResponse;
    try {
      errorResponse = (await response.json()) as ApiErrorResponse;
    } catch {
      errorResponse = {
        error: "unknown_error",
        error_code: "UNKNOWN",
        message: `Request failed with status ${String(response.status)}`,
      };
    }
    throw new ApiError(response.status, errorResponse);
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
