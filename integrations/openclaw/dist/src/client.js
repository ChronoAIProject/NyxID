import { asNyxIdError, buildProxyUrl, computeExpiryTimestamp, isTokenFresh, mapNyxIdError, requireOAuthClient, } from "./helpers.js";
async function readError(response) {
    let data = null;
    try {
        data = await response.json();
    }
    catch {
        data = null;
    }
    const apiError = asNyxIdError(data);
    if (apiError) {
        return new Error(mapNyxIdError(apiError));
    }
    return new Error(`NyxID request failed with HTTP ${response.status}.`);
}
async function tokenRequest(fetchImpl, config, body) {
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
    return (await response.json());
}
export function buildAuthHeaders(profile) {
    if (profile.apiKey) {
        return { "X-API-Key": profile.apiKey };
    }
    if (profile.accessToken) {
        return { Authorization: `Bearer ${profile.accessToken}` };
    }
    throw new Error("No NyxID credential is available for this request.");
}
export async function exchangeAuthorizationCode(fetchImpl, config, input) {
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
    return (await tokenRequest(fetchImpl, config, body));
}
export async function refreshAccessToken(fetchImpl, config, refreshToken) {
    const body = new URLSearchParams({
        grant_type: "refresh_token",
        refresh_token: refreshToken,
    });
    return (await tokenRequest(fetchImpl, config, body));
}
export async function exchangeDelegatedToken(fetchImpl, config, accessToken) {
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
    return (await tokenRequest(fetchImpl, config, body));
}
export async function refreshDelegatedToken(fetchImpl, config, delegatedToken) {
    const response = await fetchImpl(`${config.baseUrl}/api/v1/delegation/refresh`, {
        method: "POST",
        headers: {
            Authorization: `Bearer ${delegatedToken}`,
        },
    });
    if (!response.ok) {
        throw await readError(response);
    }
    return (await response.json());
}
export async function listServices(fetchImpl, config, profile) {
    const response = await fetchImpl(`${config.baseUrl}/api/v1/proxy/services`, {
        headers: buildAuthHeaders(profile),
    });
    if (!response.ok) {
        throw await readError(response);
    }
    return (await response.json());
}
export async function proxyRequest(fetchImpl, config, input) {
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
export async function ensureBaseToken(fetchImpl, config, profile) {
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
export async function ensureDelegatedToken(fetchImpl, config, profile) {
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
        }
        catch {
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
export function getProxyCredentialProfile(profile) {
    if (profile.delegatedAccessToken) {
        return {
            accessToken: profile.delegatedAccessToken,
            accessTokenExpiresAt: profile.delegatedAccessTokenExpiresAt,
            tokenType: profile.tokenType,
        };
    }
    return profile;
}
export function describeServiceConnection(service) {
    const status = service.connected ? "connected" : "not connected";
    return `${service.slug}: ${service.name} (${service.service_category}, ${status})`;
}
export function isNyxIdApiError(error) {
    return asNyxIdError(error) !== null;
}
