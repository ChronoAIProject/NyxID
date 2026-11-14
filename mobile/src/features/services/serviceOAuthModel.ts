import { z } from "zod";
import { NYXID_APP_SCHEME } from "../../lib/env";

// services/user_token_service.rs: authorization-code states expire in 10 minutes.
export const SERVICE_OAUTH_TTL_MS = 10 * 60 * 1000;
export const serviceOAuthReturnUri = () =>
  `${NYXID_APP_SCHEME}://services/oauth-return`;
export const pendingOAuthSchema = z
  .object({
    accountId: z.string().min(1),
    connectionId: z.string().min(1),
    providerId: z.string().min(1),
    catalogSlug: z.string(),
    catalogName: z.string(),
    baselineStatus: z.string(),
    baselineAuthorizedAt: z.string().nullable(),
    startedAt: z.number().finite(),
    expiresAt: z.number().finite(),
  })
  .strict();
export type PendingServiceOAuth = z.infer<typeof pendingOAuthSchema>;
export type OAuthReturn =
  | "success"
  | "access_denied"
  | "provider_error"
  | "session_mismatch"
  | "exchange_failed";
export type OAuthOutcome =
  | "success"
  | "pending"
  | "unconfirmed"
  | "declined"
  | "failed"
  | "session_mismatch";

export function isServiceOAuthReturn(url: string, scheme: string): boolean {
  // URL normalizes protocol casing. Check the raw prefix first so a lookalike
  // or uppercase scheme cannot be treated as the exact registered destination.
  if (!url.startsWith(`${scheme}://`)) return false;
  try {
    const parsed = new URL(url);
    return (
      parsed.protocol === `${scheme}:` &&
      parsed.host === "services" &&
      parsed.pathname === "/oauth-return" &&
      !parsed.username &&
      !parsed.password &&
      !parsed.hash
    );
  } catch {
    return false;
  }
}

export function parseServiceOAuthReturn(
  url: string,
  scheme: string,
): OAuthReturn | undefined {
  if (!isServiceOAuthReturn(url, scheme)) return;
  const query = new URL(url).searchParams;
  if (query.get("status") === "success") return "success";
  const code = query.get("code");
  if (
    query.get("status") === "error" &&
    [
      "access_denied",
      "provider_error",
      "session_mismatch",
      "exchange_failed",
    ].includes(code ?? "")
  )
    return code as OAuthReturn;
  return "provider_error";
}

export function evaluateServiceOAuth(
  attempt: Pick<PendingServiceOAuth, "baselineStatus" | "baselineAuthorizedAt">,
  connection:
    | { status: string; last_authorized_at?: string | null }
    | undefined,
  returned?: OAuthReturn,
): OAuthOutcome {
  if (
    connection?.status === "active" &&
    (attempt.baselineStatus !== "active" ||
      (Boolean(connection.last_authorized_at) &&
        connection.last_authorized_at !== attempt.baselineAuthorizedAt))
  )
    return "success";
  if (returned === "session_mismatch") return "session_mismatch";
  if (returned === "access_denied") return "declined";
  if (
    returned === "provider_error" ||
    returned === "exchange_failed" ||
    connection?.status === "failed"
  )
    return "failed";
  return returned === "success" ? "unconfirmed" : "pending";
}

export const oauthOutcomeCopy: Record<OAuthOutcome, string> = {
  success: "Authorization completed. Your connection is ready.",
  pending:
    "Authorization isn't finished. Continue authorization or check again.",
  unconfirmed:
    "We couldn't confirm authorization yet. Check again in a moment.",
  declined:
    "Authorization was declined. Retry when you're ready to grant access.",
  failed: "Authorization failed. Try authorizing this connection again.",
  session_mismatch:
    "Your browser is signed in to a different NyxID account. Sign out of NyxID in that browser, then retry with this account.",
};
