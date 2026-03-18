import { createHash, randomBytes } from "node:crypto";

import type { NyxIdApiError, NyxIdPluginConfig, TokenProfile, ToolContext } from "./types.js";

const DEFAULT_SCOPE = "openid profile email";
const DEFAULT_DELEGATION_SCOPE = "proxy:*";
const EXPIRY_SKEW_SECONDS = 30;

export function normalizeBaseUrl(input: string): string {
  return input.replace(/\/+$/, "");
}

export function normalizeConfig(input: Partial<NyxIdPluginConfig> | undefined): NyxIdPluginConfig {
  if (!input?.baseUrl || !input.clientId) {
    throw new Error("NyxID plugin requires both baseUrl and clientId.");
  }

  return {
    baseUrl: normalizeBaseUrl(input.baseUrl),
    clientId: input.clientId,
    clientSecret: input.clientSecret,
    defaultScopes: input.defaultScopes?.trim() || DEFAULT_SCOPE,
    delegationScopes: input.delegationScopes?.trim() || DEFAULT_DELEGATION_SCOPE,
  };
}

export function createPkcePair(): { verifier: string; challenge: string } {
  const verifier = randomBytes(32).toString("base64url");
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  return { verifier, challenge };
}

export function createOpaqueState(): string {
  return randomBytes(16).toString("hex");
}

export function buildAuthorizeUrl(
  config: NyxIdPluginConfig,
  input: { redirectUri: string; state: string; challenge: string; scope?: string },
): string {
  const url = new URL("/oauth/authorize", config.baseUrl);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("client_id", config.clientId);
  url.searchParams.set("redirect_uri", input.redirectUri);
  url.searchParams.set("scope", input.scope?.trim() || config.defaultScopes || DEFAULT_SCOPE);
  url.searchParams.set("state", input.state);
  url.searchParams.set("code_challenge", input.challenge);
  url.searchParams.set("code_challenge_method", "S256");
  return url.toString();
}

export function buildProxyUrl(baseUrl: string, service: string, apiPath: string): string {
  const trimmedPath = apiPath.replace(/^\/+/, "");
  return `${normalizeBaseUrl(baseUrl)}/api/v1/proxy/s/${encodeURIComponent(service)}/${trimmedPath}`;
}

export function decodeJwtPayload(token: string): Record<string, unknown> | null {
  const parts = token.split(".");
  if (parts.length < 2) {
    return null;
  }

  try {
    const payload = Buffer.from(parts[1], "base64url").toString("utf8");
    return JSON.parse(payload) as Record<string, unknown>;
  } catch {
    return null;
  }
}

export function isTokenFresh(token: string | undefined, expiresAt: number | undefined): boolean {
  if (!token) {
    return false;
  }

  if (typeof expiresAt === "number") {
    return expiresAt > nowEpochSeconds() + EXPIRY_SKEW_SECONDS;
  }

  const payload = decodeJwtPayload(token);
  if (!payload || typeof payload.exp !== "number") {
    return true;
  }

  return payload.exp > nowEpochSeconds() + EXPIRY_SKEW_SECONDS;
}

export function computeExpiryTimestamp(expiresInSeconds: number): number {
  return nowEpochSeconds() + expiresInSeconds;
}

export function nowEpochSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

export function mapNyxIdError(error: NyxIdApiError): string {
  switch (error.error_code) {
    case 1000:
      return `NyxID rejected the request: ${error.message}`;
    case 1001:
      return "NyxID authentication failed. Reconnect the NyxID account or refresh the token.";
    case 1002:
      return `NyxID denied the request: ${error.message}`;
    case 3000:
      return "NyxID rejected the PKCE verifier for this OAuth flow.";
    case 3001:
      return "NyxID rejected the redirect URI configured for this OpenClaw plugin.";
    case 3002:
      return `NyxID rejected the requested scope: ${error.message}`;
    case 7000:
      return "NyxID requires user approval before this action can continue.";
    case 8003:
      return "NyxID could not complete the node-backed proxy request.";
    default:
      return error.message || "NyxID returned an unknown error.";
  }
}

export function asNyxIdError(value: unknown): NyxIdApiError | null {
  if (!value || typeof value !== "object") {
    return null;
  }

  const candidate = value as Partial<NyxIdApiError>;
  if (
    typeof candidate.error === "string" &&
    typeof candidate.error_code === "number" &&
    typeof candidate.message === "string"
  ) {
    return candidate as NyxIdApiError;
  }

  return null;
}

export async function loadProfile(context: ToolContext): Promise<TokenProfile> {
  const profileFromAuth = context.auth?.profile;
  if (profileFromAuth) {
    return profileFromAuth;
  }

  const fromContext = await context.getProviderProfile?.("nyxid");
  if (fromContext) {
    return fromContext;
  }

  const fromAuthGetter = await context.auth?.getProviderProfile?.("nyxid");
  if (fromAuthGetter) {
    return fromAuthGetter;
  }

  const envAccessToken = context.env?.NYXID_ACCESS_TOKEN;
  if (envAccessToken) {
    return { accessToken: envAccessToken, tokenType: "Bearer" };
  }

  throw new Error("No NyxID auth profile is available. Connect NyxID first.");
}

export async function saveProfile(context: ToolContext, profile: TokenProfile): Promise<void> {
  await context.saveProviderProfile?.("nyxid", profile);
  await context.auth?.saveProfile?.(profile);
  if (context.auth?.profile) {
    context.auth.profile = profile;
  }
}
