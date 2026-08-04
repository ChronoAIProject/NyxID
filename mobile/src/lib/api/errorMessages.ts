import { ApiError, isApiError } from "./ApiError";

/**
 * Central error message config.
 *
 * Maps backend `error_code` → user-friendly mobile copy.
 * Add entries here instead of scattering error strings across screens.
 *
 * Error code ranges (from backend AppError):
 *   1000-1008  General (bad_request, unauthorized, forbidden, etc.)
 *   2000-2002  Auth (authentication_failed, token_expired, mfa_required)
 *   3000-3004  OAuth (pkce, redirect_uri, scope, consent, grant_type)
 *   4000-4007  RBAC (role, group, consent, slug, hierarchy)
 *   5000-5001  Service accounts
 *   6000-6005  Social auth + external providers
 *   7000-7001  Approvals
 *   8000-8003  Nodes
 *   9000-9002  API key scope
 */
const ERROR_MESSAGES: Record<number, string> = {
  // ── General ──
  1001: "Your session has expired. Please sign in again.",
  1005: "Too many requests. Please wait a moment and try again.",

  // ── Auth ──
  2000: "Invalid email or password.",
  2001: "Your session has expired. Please sign in again.",

  // ── Approvals ──
  7000: "Approval required. A notification has been sent.",
  7001: "Approval was denied or timed out.",

  // ── Nodes ──
  8001: "The credential node is offline. Try again later.",
  8002: "Request timed out waiting for the credential node.",
};

/**
 * For generic error codes (e.g. 1000 = bad_request) where multiple distinct
 * errors share the same code, match on a keyword in the server message.
 */
const BAD_REQUEST_OVERRIDES: Array<{ pattern: string; message: string }> = [
  {
    pattern: "at least one enabled notification channel",
    message:
      "Either Push or Telegram must stay enabled to receive approval requests.",
  },
];

/**
 * Resolve a user-friendly error message from any thrown error.
 *
 * Priority:
 *   1. Exact match on `error_code` in ERROR_MESSAGES
 *   2. For generic codes, keyword match in BAD_REQUEST_OVERRIDES
 *   3. The server's own `message` field (for ApiError)
 *   4. Raw Error.message fallback
 *   5. Generic fallback string
 */
export function resolveErrorMessage(error: unknown): string {
  if (!isApiError(error)) {
    if (error instanceof Error) return error.message;
    return "Something went wrong. Please try again.";
  }

  // 1. Exact code match
  const mapped = ERROR_MESSAGES[error.errorCode];
  if (mapped) return mapped;

  // 2. Keyword match for generic codes (bad_request = 1000, validation = 1008)
  if (error.errorCode === 1000 || error.errorCode === 1008) {
    const serverMsg = error.message.toLowerCase();
    for (const override of BAD_REQUEST_OVERRIDES) {
      if (serverMsg.includes(override.pattern)) {
        return override.message;
      }
    }
  }

  // 3. Server message (strip "Bad request: " prefix from thiserror Display impl)
  const cleaned = error.message.replace(/^Bad request:\s*/i, "");
  if (cleaned) return cleaned;

  return "Something went wrong. Please try again.";
}
