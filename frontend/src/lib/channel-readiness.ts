import { ApiError } from "@/lib/api-client";
import type { ApiKey } from "@/types/api";

// Verification and auth errors defined in backend/src/errors/mod.rs.
const VERIFICATION_ERROR_CODES: ReadonlySet<number> = new Set([
  1000, 1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 2000, 2001, 10000,
  10001, 10003, 10005,
]);

export function verificationErrorMessage(error: unknown): string {
  if (
    error instanceof ApiError &&
    typeof error.errorCode === "number" &&
    VERIFICATION_ERROR_CODES.has(error.errorCode) &&
    typeof error.errorResponse.message === "string" &&
    error.errorResponse.message.trim().length > 0
  )
    return error.errorResponse.message;
  const status =
    error instanceof ApiError ? ` (HTTP ${String(error.status)})` : "";
  return `Credential verification could not complete${status}. No usable verification result was received. Retry the check; if it keeps failing, ask your administrator to inspect server diagnostics.`;
}

export function routeAgentIssue(
  key: ApiKey,
  ownerOrgId: string | null,
  now = Date.now(),
): string | null {
  const source = key.credential_source;
  if (
    source &&
    (source.type === "org" ? source.org_id !== ownerOrgId : ownerOrgId !== null)
  )
    return "Different owner";
  if (!key.is_active) return "Inactive";
  if (key.expires_at && !(Date.parse(key.expires_at) > now)) return "Expired";
  if (key.platform === "nyxid-assistant") return "Assistant chat key";
  if (!key.callback_url?.trim()) return "No callback URL";
  return null;
}
