import { z } from "zod";
import {
  OAUTH_ERROR_CODES,
  OAUTH_FLOW_TOKENS,
  type OAuthAckMessage,
  type OAuthActionMessage,
  type OAuthResultMessage,
  type OAuthRetryMessage,
} from "@/types/oauth-popup";

const UUID_V4_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

export const oauthAttemptNonceSchema = z.string().regex(UUID_V4_RE);

export const oauthCompletionSearchSchema = z.object({
  status: z.enum(["complete", "error"]).optional().catch(undefined),
  flow: z.enum(OAUTH_FLOW_TOKENS).optional().catch(undefined),
  code: z.enum(OAUTH_ERROR_CODES).optional().catch(undefined),
  nonce: oauthAttemptNonceSchema.optional().catch(undefined),
});

export type OAuthCompletionSearch = z.infer<typeof oauthCompletionSearchSchema>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function isOAuthResultMessage(
  value: unknown,
): value is OAuthResultMessage {
  if (!isRecord(value) || value.type !== "oauth_result") return false;
  return (
    (value.status === "complete" || value.status === "error") &&
    (value.flow === undefined ||
      OAUTH_FLOW_TOKENS.includes(value.flow as never)) &&
    (value.code === undefined ||
      OAUTH_ERROR_CODES.includes(value.code as never))
  );
}

export function isOAuthActionMessage(
  value: unknown,
): value is OAuthActionMessage {
  return (
    isRecord(value) &&
    value.type === "oauth_action" &&
    (value.action === "view_result" ||
      value.action === "retry" ||
      value.action === "dismiss")
  );
}

export function isOAuthAckMessage(value: unknown): value is OAuthAckMessage {
  return isRecord(value) && value.type === "oauth_ack";
}

export function isOAuthRetryMessage(
  value: unknown,
  expectedProviderOrigin?: string,
): value is OAuthRetryMessage {
  return (
    isRecord(value) &&
    value.type === "oauth_retry" &&
    typeof value.nextNonce === "string" &&
    oauthAttemptNonceSchema.safeParse(value.nextNonce).success &&
    typeof value.url === "string" &&
    validateAuthorizationUrl(
      value.url,
      value.nextNonce,
      expectedProviderOrigin,
    ) !== null
  );
}

export function validateAuthorizationUrl(
  rawUrl: string,
  nonce: string,
  expectedProviderOrigin?: string,
): URL | null {
  if (!oauthAttemptNonceSchema.safeParse(nonce).success) return null;
  const url = validateHttpAuthorizationUrl(rawUrl);
  if (
    url === null ||
    (expectedProviderOrigin !== undefined &&
      url.origin !== expectedProviderOrigin) ||
    url.searchParams.get("state") !== `1cc_${nonce}`
  ) {
    return null;
  }
  return url;
}

/** Safe manual fallback when an older backend has no popup nonce contract. */
export function validateHttpAuthorizationUrl(rawUrl: string): URL | null {
  try {
    const url = new URL(rawUrl, window.location.origin);
    if (
      (url.protocol !== "https:" && url.protocol !== "http:") ||
      url.username !== "" ||
      url.password !== ""
    ) {
      return null;
    }
    return url;
  } catch {
    return null;
  }
}
