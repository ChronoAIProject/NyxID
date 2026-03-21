import {
  asNyxIdError,
  buildProxyUrl,
  computeExpiryTimestamp,
  isTokenFresh,
  mapNyxIdError,
  requireOAuthClient,
} from "./helpers.js";
import type {
  DelegatedTokenSet,
  NyxIdApiError,
  NyxIdPluginConfig,
  OAuthTokenSet,
  ServiceListResponse,
  TokenProfile,
} from "./types.js";

type FetchImpl = typeof fetch;

async function readError(response: Response): Promise<Error> {
  let data: unknown = null;
  try {
    data = await response.json();
  } catch {
    data = null;
  }

  const apiError = asNyxIdError(data);
  if (apiError) {
    return new Error(mapNyxIdError(apiError));
  }

  return new Error(`NyxID request failed with HTTP ${response.status}.`);
}

async function tokenRequest(fetchImpl: FetchImpl, config: NyxIdPluginConfig, body: URLSearchParams): Promise<OAuthTokenSet | DelegatedTokenSet> {
  const response = await fetchImpl(`${config.baseUrl}/oauth/token`, {
    method: "POST",
    headers: {
      "Content-Type": "application/x-www-form-urlencoded",
    },
    body,
  });

  if (!response.ok) {
    throw await readError(response);
  }

  return (await response.json()) as OAuthTokenSet | DelegatedTokenSet;
}

export function buildAuthHeaders(profile: TokenProfile): Record<string, string> {
  if (profile.apiKey) {
    return { "X-API-Key": profile.apiKey };
  }

  if (profile.accessToken) {
    return { Authorization: `Bearer ${profile.accessToken}` };
  }

  throw new Error("No NyxID credential is available for this request.");
}

export async function exchangeAuthorizationCode(
  fetchImpl: FetchImpl,
  config: NyxIdPluginConfig,
  input: { code: string; redirectUri: string; codeVerifier: string },
): Promise<OAuthTokenSet> {
  requireOAuthClient(config);

  const body = new URLSearchParams({
    grant_type: "authorization_code",
    code: input.code,
    redirect_uri: input.redirectUri,
    client_id: config.clientId,
    code_verifier: input.codeVerifier,
  });

  if (config.clientSecret) {
    body.set("client_secret", config.clientSecret);
  }

  return (await tokenRequest(fetchImpl, config, body)) as OAuthTokenSet;
}

export async function refreshAccessToken(
  fetchImpl: FetchImpl,
  config: NyxIdPluginConfig,
  refreshToken: string,
): Promise<OAuthTokenSet> {
  const body = new URLSearchParams({
    grant_type: "refresh_token",
    refresh_token: refreshToken,
  });

  return (await tokenRequest(fetchImpl, config, body)) as OAuthTokenSet;
}

export async function exchangeDelegatedToken(
  fetchImpl: FetchImpl,
  config: NyxIdPluginConfig,
  accessToken: string,
): Promise<DelegatedTokenSet> {
  requireOAuthClient(config);

  if (!config.clientSecret) {
    throw new Error("NyxID delegated proxy access requires clientSecret because RFC 8693 token exchange is confidential-client only.");
  }

  const body = new URLSearchParams({
    grant_type: "urn:ietf:params:oauth:grant-type:token-exchange",
    client_id: config.clientId,
    client_secret: config.clientSecret,
    subject_token: accessToken,
    subject_token_type: "urn:ietf:params:oauth:token-type:access_token",
    scope: config.delegationScopes || "proxy:*",
  });

  return (await tokenRequest(fetchImpl, config, body)) as DelegatedTokenSet;
}

export async function refreshDelegatedToken(
  fetchImpl: FetchImpl,
  config: NyxIdPluginConfig,
  delegatedToken: string,
): Promise<DelegatedTokenSet> {
  const response = await fetchImpl(`${config.baseUrl}/api/v1/delegation/refresh`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${delegatedToken}`,
    },
  });

  if (!response.ok) {
    throw await readError(response);
  }

  return (await response.json()) as DelegatedTokenSet;
}

export async function listServices(
  fetchImpl: FetchImpl,
  config: NyxIdPluginConfig,
  profile: TokenProfile,
): Promise<ServiceListResponse> {
  const response = await fetchImpl(`${config.baseUrl}/api/v1/proxy/services`, {
    headers: buildAuthHeaders(profile),
  });

  if (!response.ok) {
    throw await readError(response);
  }

  return (await response.json()) as ServiceListResponse;
}

export async function proxyRequest(
  fetchImpl: FetchImpl,
  config: NyxIdPluginConfig,
  input: {
    profile: TokenProfile;
    service: string;
    method: string;
    path: string;
    body?: unknown;
  },
): Promise<unknown> {
  const response = await fetchImpl(buildProxyUrl(config.baseUrl, input.service, input.path), {
    method: input.method,
    headers: {
      ...buildAuthHeaders(input.profile),
      "Content-Type": "application/json",
    },
    body: input.body === undefined ? undefined : JSON.stringify(input.body),
  });

  if (!response.ok) {
    throw await readError(response);
  }

  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    return response.json();
  }

  return response.text();
}

export async function ensureBaseToken(
  fetchImpl: FetchImpl,
  config: NyxIdPluginConfig,
  profile: TokenProfile,
): Promise<TokenProfile> {
  if (profile.apiKey) {
    return profile;
  }

  if (isTokenFresh(profile.accessToken, profile.accessTokenExpiresAt)) {
    return profile;
  }

  if (!profile.refreshToken) {
    throw new Error("NyxID access token has expired and no refresh token is available.");
  }

  const refreshed = await refreshAccessToken(fetchImpl, config, profile.refreshToken);
  return {
    ...profile,
    accessToken: refreshed.access_token,
    refreshToken: refreshed.refresh_token ?? profile.refreshToken,
    accessTokenExpiresAt: computeExpiryTimestamp(refreshed.expires_in),
    tokenType: refreshed.token_type,
    scope: refreshed.scope,
  };
}

export async function ensureDelegatedToken(
  fetchImpl: FetchImpl,
  config: NyxIdPluginConfig,
  profile: TokenProfile,
): Promise<TokenProfile> {
  const baseProfile = await ensureBaseToken(fetchImpl, config, profile);

  if (baseProfile.apiKey) {
    return baseProfile;
  }

  if (!config.clientId || !config.clientSecret) {
    return baseProfile;
  }

  if (isTokenFresh(baseProfile.delegatedAccessToken, baseProfile.delegatedAccessTokenExpiresAt)) {
    return baseProfile;
  }

  if (baseProfile.delegatedAccessToken) {
    try {
      const refreshedDelegated = await refreshDelegatedToken(fetchImpl, config, baseProfile.delegatedAccessToken);
      return {
        ...baseProfile,
        delegatedAccessToken: refreshedDelegated.access_token,
        delegatedAccessTokenExpiresAt: computeExpiryTimestamp(refreshedDelegated.expires_in),
      };
    } catch {
      // Fall back to a fresh exchange from the user access token.
    }
  }

  if (!baseProfile.accessToken) {
    return baseProfile;
  }

  const exchanged = await exchangeDelegatedToken(fetchImpl, config, baseProfile.accessToken);
  return {
    ...baseProfile,
    delegatedAccessToken: exchanged.access_token,
    delegatedAccessTokenExpiresAt: computeExpiryTimestamp(exchanged.expires_in),
  };
}

export function getProxyCredentialProfile(profile: TokenProfile): TokenProfile {
  if (profile.delegatedAccessToken) {
    return {
      accessToken: profile.delegatedAccessToken,
      accessTokenExpiresAt: profile.delegatedAccessTokenExpiresAt,
      tokenType: profile.tokenType,
    };
  }

  return profile;
}

export function describeServiceConnection(service: ServiceListResponse["services"][number]): string {
  const status = service.connected ? "connected" : "not connected";
  return `${service.slug}: ${service.name} (${service.service_category}, ${status})`;
}

export function isNyxIdApiError(error: unknown): error is NyxIdApiError {
  return asNyxIdError(error) !== null;
}
