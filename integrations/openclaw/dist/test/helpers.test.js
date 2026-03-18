import test from "node:test";
import assert from "node:assert/strict";
import { buildAuthorizeUrl, buildProxyUrl, computeExpiryTimestamp, isTokenFresh, mapNyxIdError, normalizeBaseUrl, normalizeConfig, } from "../src/helpers.js";
import { buildAuthHeaders, getProxyCredentialProfile } from "../src/client.js";
test("normalizeBaseUrl removes trailing slashes", () => {
    assert.equal(normalizeBaseUrl("https://nyx-api.chrono-ai.fun///"), "https://nyx-api.chrono-ai.fun");
});
test("normalizeConfig uses hosted NyxID default base url", () => {
    assert.equal(normalizeConfig(undefined).baseUrl, "https://nyx-api.chrono-ai.fun");
});
test("buildAuthorizeUrl produces PKCE-friendly NyxID authorize URL", () => {
    const url = buildAuthorizeUrl({
        baseUrl: "https://nyx-api.chrono-ai.fun",
        clientId: "client-123",
        defaultScopes: "openid profile email",
        delegationScopes: "proxy:*",
    }, {
        redirectUri: "https://openclaw.local/callback",
        state: "state-1",
        challenge: "challenge-1",
    });
    assert.match(url, /^https:\/\/nyx-api\.chrono-ai\.fun\/oauth\/authorize\?response_type=code&client_id=client-123/);
    assert.match(url, /code_challenge=challenge-1/);
    assert.match(url, /code_challenge_method=S256/);
});
test("buildProxyUrl normalizes service path", () => {
    assert.equal(buildProxyUrl("https://nyx-api.chrono-ai.fun/", "twitter", "/2/tweets"), "https://nyx-api.chrono-ai.fun/api/v1/proxy/s/twitter/2/tweets");
});
test("computeExpiryTimestamp returns a future epoch time", () => {
    const now = Math.floor(Date.now() / 1000);
    const exp = computeExpiryTimestamp(60);
    assert.ok(exp >= now + 60);
});
test("isTokenFresh respects explicit expiry timestamps", () => {
    assert.equal(isTokenFresh("token", Math.floor(Date.now() / 1000) + 120), true);
    assert.equal(isTokenFresh("token", Math.floor(Date.now() / 1000) - 5), false);
});
test("mapNyxIdError returns user-facing approval guidance", () => {
    assert.equal(mapNyxIdError({
        error: "approval_required",
        error_code: 7000,
        message: "Approval required",
    }), "NyxID requires user approval before this action can continue.");
});
test("buildAuthHeaders prefers X-API-Key when api key is present", () => {
    assert.deepEqual(buildAuthHeaders({ apiKey: "nyx_k_123" }), {
        "X-API-Key": "nyx_k_123",
    });
});
test("getProxyCredentialProfile prefers delegated tokens but falls back to base profile", () => {
    assert.deepEqual(getProxyCredentialProfile({
        accessToken: "base",
        delegatedAccessToken: "delegated",
        delegatedAccessTokenExpiresAt: 123,
    }), {
        accessToken: "delegated",
        accessTokenExpiresAt: 123,
        tokenType: undefined,
    });
    assert.deepEqual(getProxyCredentialProfile({
        apiKey: "nyx_k_123",
    }), {
        apiKey: "nyx_k_123",
    });
});
