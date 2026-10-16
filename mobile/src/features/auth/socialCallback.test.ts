import assert from "node:assert/strict";
import { test } from "node:test";
import { parseSocialCallback, SOCIAL_CALLBACK_URL } from "./socialCallback";

test("Expo launch and unrelated native links are ignored by social login", () => {
  for (const url of [
    "exp://127.0.0.1:8081",
    "exp://127.0.0.1:8081/--/login/device?user_code=ABCD-EFGH",
    "nyxid://account/security",
    "nyxid://login/device?user_code=ABCD-EFGH",
    "nyxid://login/agent-key?user_code=ABCD-EFGH",
    "not a URL",
  ]) {
    assert.equal(parseSocialCallback(url), null);
  }
});

test("social callbacks require the exact scheme, authority and path", () => {
  for (const url of [
    "https://auth/social/callback?status=success",
    "nyxid://other/social/callback?status=success",
    "nyxid://auth/social/callback-extra?status=success",
    "nyxid://auth/social/callback/extra?status=success",
    "nyxid://person@auth/social/callback?status=success",
    "nyxid://auth:8081/social/callback?status=success",
  ]) {
    assert.equal(parseSocialCallback(url), null);
  }
});

test("the actual native callback preserves successful session fields", () => {
  assert.deepEqual(
    parseSocialCallback(
      `${SOCIAL_CALLBACK_URL}?status=success&provider=github&access_token=synthetic-access&refresh_token=synthetic-refresh&expires_in=3600`,
    ),
    {
      status: "success",
      provider: "github",
      accessToken: "synthetic-access",
      refreshToken: "synthetic-refresh",
      expiresIn: 3600,
    },
  );
});

test("errors and invalid status on the actual callback still produce feedback", () => {
  assert.deepEqual(
    parseSocialCallback(
      `${SOCIAL_CALLBACK_URL}?status=error&provider=apple&error=social_auth_denied`,
    ),
    { status: "error", provider: "apple", error: "social_auth_denied" },
  );
  assert.deepEqual(parseSocialCallback(SOCIAL_CALLBACK_URL), {
    status: "error",
    provider: undefined,
    error: "social_auth_unknown",
  });
});
