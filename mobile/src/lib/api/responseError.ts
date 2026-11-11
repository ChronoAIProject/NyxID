import { ApiError } from "./ApiError";
import { mfaChallengeFromResponse } from "../auth/mfaChallenge";

export type ErrorHeaders = { get(name: string): string | null };
export const TEMPORARILY_UNAVAILABLE =
  "NyxID is temporarily unavailable. Try again in a moment.";
export const CONNECTION_UNAVAILABLE =
  "Couldn't reach NyxID. Check your connection and try again.";

function diagnosticId(value: string | null | undefined): string | undefined {
  return value && /^[A-Za-z0-9._:-]{1,128}$/.test(value) ? value : undefined;
}

export function errorDiagnostics(headers?: ErrorHeaders) {
  return {
    requestId: diagnosticId(headers?.get("x-request-id")),
    traceId: diagnosticId(headers?.get("X-NyxID-Trace-Id")),
  };
}

/** Error bodies are data only when they match NyxID's structured contract.
 * Legacy error-only responses remain supported. Never retain proxy bodies. */
export function buildApiError(
  payload: unknown,
  status: number,
  headers?: ErrorHeaders,
): ApiError {
  const diagnostics = errorDiagnostics(headers);
  const challenge = mfaChallengeFromResponse(payload, status);
  if (challenge) return Object.assign(challenge, diagnostics);
  if (payload && typeof payload === "object" && !Array.isArray(payload)) {
    const obj = payload as Record<string, unknown>;
    const legacy =
      obj.error == null &&
      Number.isInteger(obj.error_code) &&
      Number(obj.error_code) >= 0 &&
      typeof obj.message === "string";
    const symbolic =
      typeof obj.error === "string" && /^[a-z][a-z0-9_]*$/.test(obj.error);
    if (
      (symbolic || legacy) &&
      (obj.message === undefined || typeof obj.message === "string") &&
      (obj.error_code === undefined || typeof obj.error_code === "number")
    ) {
      const reason =
        obj.details &&
        typeof obj.details === "object" &&
        (obj.details as Record<string, unknown>).reason ===
          "mfa_challenge_expired"
          ? "mfa_challenge_expired"
          : undefined;
      return new ApiError({
        errorKey: symbolic ? String(obj.error) : `request_failed_${status}`,
        errorCode: typeof obj.error_code === "number" ? obj.error_code : 0,
        statusCode: status,
        message:
          typeof obj.message === "string" && obj.message.length
            ? obj.message
            : typeof obj.error_description === "string" &&
                obj.error_description.length
              ? obj.error_description
              : symbolic
                ? String(obj.error)
                : `request_failed_${status}`,
        reason,
        details: obj.details,
        ...diagnostics,
      });
    }
  }
  const message =
    status === 0
      ? CONNECTION_UNAVAILABLE
      : status >= 500
        ? TEMPORARILY_UNAVAILABLE
        : status === 429
          ? "Too many requests. Please wait a moment and try again."
          : status === 401
            ? "Your session has expired. Please sign in again."
            : "NyxID couldn't complete this request. Please try again.";
  return new ApiError({
    errorKey: status === 0 ? "network_error" : "invalid_error_response",
    errorCode: 0,
    statusCode: status,
    message,
    ...diagnostics,
  });
}

/** Called only around network I/O, never around an authority check or mutation. */
export function networkApiError(cause: unknown): Error {
  if (
    cause &&
    typeof cause === "object" &&
    "name" in cause &&
    cause.name === "AbortError"
  )
    return cause as Error;
  return buildApiError(null, 0);
}
