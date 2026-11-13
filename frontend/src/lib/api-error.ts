import type { ApiErrorResponse } from "@/types/api";
import {
  INSUFFICIENT_CREDITS,
  isInsufficientCreditsHttp,
} from "./credits-denial";

export type ErrorFormat = "nyxid" | "cma";
const symbol = (value: unknown): value is string =>
  typeof value === "string" && /^[a-z][a-z0-9_]*$/.test(value);

type ErrorHeaders = { get(name: string): string | null };
const diagnosticId = (value: string | null | undefined) =>
  value && /^[A-Za-z0-9._:-]{1,128}$/.test(value) ? value : undefined;

export class ApiError extends Error {
  readonly status: number;
  readonly errorCode: number;
  readonly errorResponse: ApiErrorResponse;
  readonly requestId?: string;
  readonly traceId?: string;

  constructor(
    status: number,
    response: ApiErrorResponse,
    headers?: ErrorHeaders,
  ) {
    super(response.message);
    this.name = "ApiError";
    this.status = status;
    this.errorCode = response.error_code;
    this.errorResponse = response;
    this.requestId = diagnosticId(headers?.get("x-request-id"));
    this.traceId = diagnosticId(headers?.get("X-NyxID-Trace-Id"));
  }
}

/** Preserve NyxID errors, never arbitrary JSON messages or raw proxy pages. */
export function apiErrorBody(
  payload: unknown,
  status: number,
  format: ErrorFormat = "nyxid",
): ApiErrorResponse {
  if (payload && typeof payload === "object" && !Array.isArray(payload)) {
    const obj = payload as Record<string, unknown>;
    // Older NyxID responses identify the error by numeric code alone.
    // Keep that contract, including the status-scoped credits denial fallback.
    const legacy =
      obj.error == null &&
      Number.isInteger(obj.error_code) &&
      Number(obj.error_code) >= 0 &&
      typeof obj.message === "string";
    if (
      (symbol(obj.error) || legacy) &&
      (obj.message === undefined || typeof obj.message === "string") &&
      (obj.error_code === undefined || typeof obj.error_code === "number")
    ) {
      return {
        ...obj,
        error: symbol(obj.error)
          ? obj.error
          : isInsufficientCreditsHttp(status, obj)
            ? INSUFFICIENT_CREDITS
            : "unknown_error",
        error_code: typeof obj.error_code === "number" ? obj.error_code : -1,
        message:
          typeof obj.message === "string" && obj.message.length
            ? obj.message
            : typeof obj.error_description === "string" &&
                obj.error_description.length
              ? obj.error_description
              : symbol(obj.error)
                ? obj.error
                : "unknown_error",
      };
    }
    // Only CMA callers opt into its RFC 9457 / Responses contracts. A JSON
    // object with a title or detail is not a NyxID error on unrelated routes.
    if (format === "cma") {
      const parsed = cmaErrorBody(obj, status);
      if (parsed) return parsed;
    }
  }
  return {
    error: status === 0 ? "network_error" : "unknown_error",
    error_code: -1,
    message:
      status === 0
        ? "Couldn't reach NyxID. Check your connection and try again."
        : status >= 500
          ? "NyxID is temporarily unavailable. Try again in a moment."
          : status === 429
            ? "Too many requests. Please wait a moment and try again."
            : status === 401
              ? "Your session has expired. Please sign in again."
              : "NyxID couldn't complete this request. Please try again.",
  };
}

function cmaErrorBody(
  obj: Record<string, unknown>,
  status: number,
): ApiErrorResponse | undefined {
  if (obj.status !== undefined && obj.status !== status) return;
  const detail =
    typeof obj.detail === "string" && obj.detail.trim()
      ? obj.detail
      : typeof obj.title === "string" && obj.title.trim()
        ? obj.title
        : undefined;
  if (detail && obj.status === status && symbol(obj.code)) {
    const standard = new Set([
      "type",
      "title",
      "status",
      "detail",
      "instance",
      "code",
    ]);
    const extensions = Object.fromEntries(
      Object.entries(obj).filter(([key]) => !standard.has(key)),
    );
    return {
      ...obj,
      error: symbol(obj.code) ? obj.code : "unknown_error",
      error_code: -1,
      message: detail,
      details: obj.details ?? extensions,
    };
  }
  if (obj.error && typeof obj.error === "object" && !Array.isArray(obj.error)) {
    const error = obj.error as Record<string, unknown>;
    if (
      symbol(error.code) &&
      typeof error.message === "string" &&
      error.message.trim() &&
      typeof error.type === "string" &&
      error.type.trim() &&
      (error.param === null || typeof error.param === "string")
    ) {
      return {
        ...obj,
        error: error.code,
        code: error.code,
        error_code: -1,
        message: error.message,
        details: error,
      } as ApiErrorResponse;
    }
  }
}

export async function responseApiError(
  response: Response,
  format: ErrorFormat = "nyxid",
): Promise<ApiError> {
  const payload: unknown = await response.json().catch(() => null);
  return new ApiError(
    response.status,
    apiErrorBody(payload, response.status, format),
    response.headers,
  );
}

export function networkApiError(cause: unknown): Error {
  if (
    cause &&
    typeof cause === "object" &&
    "name" in cause &&
    cause.name === "AbortError"
  )
    return cause as Error;
  // Preserve fetch's TypeError classification for existing reconnect policies.
  return Object.assign(new TypeError(apiErrorBody(null, 0).message), {
    status: 0,
  });
}
