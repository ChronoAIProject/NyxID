import { resolveAuthDeviceErrorMessage } from "../../lib/api/errorMessages";

export type LoginFailure =
  | "malformed"
  | "not_found"
  | "expired"
  | "completed"
  | "denied"
  | "rate_limited"
  | "network"
  | "server"
  | "rejected"
  | "mfa_step_up";
export class LoginRequestError extends Error {
  constructor(
    readonly kind: LoginFailure,
    readonly retryAfterMs?: number,
  ) {
    super(kind);
  }
}
export function classifyLoginError(error: unknown): LoginFailure {
  if (error instanceof LoginRequestError) return error.kind;
  const code = (error as { errorCode?: number } | null)?.errorCode;
  if ((error as { statusCode?: number } | null)?.statusCode === 429)
    return "rate_limited";
  if (((error as { statusCode?: number } | null)?.statusCode ?? 0) >= 500)
    return "server";
  if (code === 1005) return "rate_limited";
  if (code === 2003) return "mfa_step_up";
  if (code === 11201 || code === 11901) return "expired";
  if (code === 11205 || code === 11905) return "completed";
  if (code === 11204 || code === 11904) return "denied";
  if ([11203, 11206, 11903, 11906].includes(code ?? 0)) return "rate_limited";
  if ([11200, 11207, 11900, 11907].includes(code ?? 0)) return "not_found";
  const failure = error as { errorKey?: string; statusCode?: number } | null;
  if (
    failure?.statusCode === 0 ||
    (error instanceof Error && error.name === "AbortError")
  )
    return "network";
  if (failure?.errorKey === "invalid_error_response") return "server";
  if (
    failure?.statusCode &&
    failure.statusCode >= 400 &&
    failure.statusCode < 500
  )
    return "rejected";
  return "server";
}

/** Only call after dispatch: local validation cannot make a decision uncertain. */
export function isLoginDecisionUncertain(error: unknown): boolean {
  const kind = classifyLoginError(error);
  if (kind !== "network" && kind !== "server") return false;
  const failure = error as { statusCode?: number; errorKey?: string } | null;
  return (
    failure?.statusCode === undefined ||
    failure.statusCode === 0 ||
    failure.statusCode >= 500 ||
    failure.errorKey === "invalid_error_response"
  );
}
export function loginErrorCopy(error: unknown): string {
  return {
    malformed: "Enter a valid eight-character login code.",
    not_found:
      "No login request was found on {{host}}. Check the code and server.",
    expired:
      "This login request has expired. Start a new request on the requesting device.",
    completed:
      "This login request was already used. Start a new request on the requesting device.",
    denied:
      "This login request was rejected. Start a new request on the requesting device.",
    rate_limited: "Too many attempts. Wait before trying again.",
    network:
      "Could not reach the login server. Check your connection and try again.",
    server: "NyxID couldn't complete this request. Try again.",
    rejected: "NyxID couldn't complete this request. Try again.",
    mfa_step_up:
      "Verify this session before approving a sign-in on another device.",
  }[classifyLoginError(error)];
}

/** Known device codes keep their specific copy; unknown rejections are not transport failures. */
export function deviceLoginErrorCopy(error: unknown): string {
  const code = (error as { errorCode?: number } | null)?.errorCode;
  return classifyLoginError(error) === "rejected" &&
    code !== undefined &&
    code >= 11200 &&
    code <= 11207
    ? resolveAuthDeviceErrorMessage(error)
    : loginErrorCopy(error);
}
