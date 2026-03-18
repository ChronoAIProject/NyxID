import test from "node:test";
import assert from "node:assert/strict";

import {
  buildAuthorizeUrl,
  buildProxyUrl,
  computeExpiryTimestamp,
  isTokenFresh,
  mapNyxIdError,
  normalizeBaseUrl,
} from "../src/helpers.js";

test("normalizeBaseUrl removes trailing slashes", () => {
  assert.equal(normalizeBaseUrl("https://auth.nyxid.dev///"), "https://auth.nyxid.dev");
});

test("buildAuthorizeUrl produces PKCE-friendly NyxID authorize URL", () => {
  const url = buildAuthorizeUrl(
    {
      baseUrl: "https://auth.nyxid.dev",
      clientId: "client-123",
      defaultScopes: "openid profile email",
      delegationScopes: "proxy:*",
    },
    {
      redirectUri: "https://openclaw.local/callback",
      state: "state-1",
      challenge: "challenge-1",
    },
  );

  assert.match(
    url,
    /^https:\/\/auth\.nyxid\.dev\/oauth\/authorize\?response_type=code&client_id=client-123/,
  );
  assert.match(url, /code_challenge=challenge-1/);
  assert.match(url, /code_challenge_method=S256/);
});

test("buildProxyUrl normalizes service path", () => {
  assert.equal(
    buildProxyUrl("https://auth.nyxid.dev/", "twitter", "/2/tweets"),
    "https://auth.nyxid.dev/api/v1/proxy/s/twitter/2/tweets",
  );
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
  assert.equal(
    mapNyxIdError({
      error: "approval_required",
      error_code: 7000,
      message: "Approval required",
    }),
    "NyxID requires user approval before this action can continue.",
  );
});
