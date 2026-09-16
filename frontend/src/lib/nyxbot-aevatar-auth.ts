import { z } from "zod";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";

export const AEVATAR_ORIGIN = "https://aevatar-console-backend-api.aevatar.ai";
export const AEVATAR_CHANNELS_URL = `${AEVATAR_ORIGIN}/channels`;
const REDIRECT_URI = `${AEVATAR_ORIGIN}/auto/callback`;
const SCOPE = "openid profile email proxy";
const authConfigSchema = z.object({
  clientId: z.string().uuid(),
  baseUrl: z.literal("https://nyx-api.chrono-ai.fun"),
});
const tokenSchema = z.object({
  access_token: z.string().min(1),
  token_type: z.string().regex(/^bearer$/i),
  expires_in: z.number().positive(),
});

export class AevatarAuthError extends Error {
  readonly code: "channelAuthRequired" | "channelConsentRequired";

  constructor(code: "channelAuthRequired" | "channelConsentRequired") {
    super(code);
    this.name = "AevatarAuthError";
    this.code = code;
  }
}

type Access = { userId: string; token: string; expiresAt: number };
let cached: Access | undefined;
let pending: { userId: string; promise: Promise<Access> } | undefined;
let identityVersion = 0;

// Only memory holds the access token. Never store refresh tokens, OAuth codes,
// or account credentials in local/session storage or the query cache.
useAuthStore.subscribe((state, previous) => {
  if (
    state.user?.id !== previous.user?.id ||
    state.isAuthenticated !== previous.isAuthenticated
  ) {
    cached = undefined;
    pending = undefined;
    identityVersion++;
  }
});

function base64url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

async function authorize(userId: string): Promise<Access> {
  const config = authConfigSchema.parse(
    await apiClient("/proxy/s/aevatar/api/auth/nyxid/config", {
      preserveSessionOn401: true,
      signal: AbortSignal.timeout(15_000),
    }),
  );
  const verifier = base64url(crypto.getRandomValues(new Uint8Array(32)));
  const state = base64url(crypto.getRandomValues(new Uint8Array(32)));
  const challenge = base64url(
    new Uint8Array(
      await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)),
    ),
  );
  const query = new URLSearchParams({
    response_type: "code",
    client_id: config.clientId,
    redirect_uri: REDIRECT_URI,
    scope: SCOPE,
    state,
    code_challenge: challenge,
    code_challenge_method: "S256",
  });
  query.append("resource", `${config.baseUrl}/api/v1/proxy/s/aevatar`);

  // NyxID's existing JSON authorization mode accepts the first-party browser
  // session and honors existing consent. No redirect, CLI token, or new client
  // registration is needed; the registered redirect is verified below.
  const authorization = await fetch(`/oauth/authorize?${query}`, {
    headers: { Accept: "application/json" },
    credentials: "include",
    cache: "no-store",
    redirect: "error",
    signal: AbortSignal.timeout(15_000),
  });
  const body: unknown = await authorization.json();
  if (!authorization.ok) {
    const error = z.object({ error: z.string() }).safeParse(body);
    throw new AevatarAuthError(
      error.success && error.data.error === "consent_required"
        ? "channelConsentRequired"
        : "channelAuthRequired",
    );
  }
  const redirect = new URL(
    z.object({ redirect_url: z.string().url() }).parse(body).redirect_url,
  );
  if (
    redirect.origin + redirect.pathname !== REDIRECT_URI ||
    redirect.username ||
    redirect.password ||
    redirect.hash ||
    redirect.searchParams.getAll("state").length !== 1 ||
    redirect.searchParams.get("state") !== state ||
    redirect.searchParams.getAll("code").length !== 1 ||
    !redirect.searchParams.get("code") ||
    redirect.searchParams.has("error") ||
    (redirect.searchParams.has("iss") &&
      redirect.searchParams.get("iss") !== config.baseUrl)
  )
    throw new AevatarAuthError("channelAuthRequired");

  const issued = await fetch("/oauth/token", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    credentials: "omit",
    cache: "no-store",
    redirect: "error",
    body: new URLSearchParams({
      grant_type: "authorization_code",
      client_id: config.clientId,
      redirect_uri: REDIRECT_URI,
      code: redirect.searchParams.get("code")!,
      code_verifier: verifier,
    }),
    signal: AbortSignal.timeout(15_000),
  });
  if (!issued.ok) throw new AevatarAuthError("channelAuthRequired");
  const token = tokenSchema.parse(await issued.json());
  // Check the authoritative subject; decoding a JWT is not verification.
  const identity = await fetch("/oauth/userinfo", {
    headers: { Authorization: `Bearer ${token.access_token}` },
    credentials: "omit",
    cache: "no-store",
    redirect: "error",
    signal: AbortSignal.timeout(15_000),
  });
  if (
    !identity.ok ||
    z.object({ sub: z.string() }).parse(await identity.json()).sub !== userId
  )
    throw new AevatarAuthError("channelAuthRequired");
  return {
    userId,
    token: token.access_token,
    expiresAt: Date.now() + Math.min(token.expires_in * 1000 - 30_000, 300_000),
  };
}

export async function getAevatarAuthorization(): Promise<string> {
  const { user, isAuthenticated } = useAuthStore.getState();
  if (!user || !isAuthenticated)
    throw new AevatarAuthError("channelAuthRequired");
  if (cached?.userId === user.id && cached.expiresAt > Date.now()) {
    return `Bearer ${cached.token}`;
  }
  const version = identityVersion;
  const attempt =
    pending?.userId === user.id
      ? pending
      : { userId: user.id, promise: authorize(user.id) };
  pending = attempt;
  try {
    const access = await attempt.promise;
    if (version !== identityVersion)
      throw new AevatarAuthError("channelAuthRequired");
    cached = access;
    return `Bearer ${access.token}`;
  } catch (error) {
    // OAuth errors can contain authorization codes or tokens; never retain them.
    throw error instanceof AevatarAuthError
      ? error
      : new AevatarAuthError("channelAuthRequired");
  } finally {
    if (pending === attempt) pending = undefined;
  }
}

export function clearAevatarAuthorization(): void {
  cached = undefined;
  pending = undefined;
  identityVersion++;
}
