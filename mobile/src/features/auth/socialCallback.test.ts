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

test("the native callback returns only a one-time code", () => {
  const code = "a".repeat(64);
  assert.deepEqual(
    parseSocialCallback(SOCIAL_CALLBACK_URL + "?status=success&provider=github&code=" + code),
    { status: "success", provider: "github", code },
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

test('Google, GitHub and Apple MFA callbacks carry only a fragment challenge', () => {
  for (const provider of ['google', 'github', 'apple']) {
    const result = parseSocialCallback(`nyxid://auth/social/callback#status=mfa_required&provider=${provider}&mfa_token=private-challenge`);
    assert.deepEqual(result, { status: 'mfa_required', provider, mfaToken: 'private-challenge' });
    assert.equal(parseSocialCallback(`nyxid://auth/social/callback?status=mfa_required&provider=${provider}&mfa_token=query-token`)?.mfaToken, undefined);
  }
});

test('social callback parser rejects foreign origins and prefix lookalikes', () => {
  for (const url of ['https://auth/social/callback#status=mfa_required', 'nyxid://auth/social/callback-extra#status=mfa_required', 'nyxid://auth.evil/social/callback#status=mfa_required', 'nyxid://other/social/callback#status=mfa_required']) {
    assert.equal(parseSocialCallback(url), null);
  }
});

test("legacy success and duplicate codes are rejected", () => {
  const code = "a".repeat(64);
  for (const query of [
    "access_token=fixture&refresh_token=fixture",
    "code=" + code + "&code=" + code,
    "code=invalid",
  ]) {
    assert.deepEqual(
      parseSocialCallback(SOCIAL_CALLBACK_URL + "?status=success&provider=apple&" + query),
      { status: "error", provider: "apple", error: "social_auth_invalid_code" },
    );
  }
});

test('MFA callbacks reject missing, query-only or malformed fragment challenges', () => {
  for (const fragment of ['', 'mfa_token=', 'mfa_token=contains%20space', `mfa_token=${'a'.repeat(129)}`, 'mfa_token=%3Cscript%3E']) {
    const result = parseSocialCallback(`nyxid://auth/social/callback?status=mfa_required&provider=google#${fragment}`);
    assert.equal(result?.status, 'error');
    assert.equal(result?.mfaToken, undefined);
  }
});

test('success and error never expose an injected fragment MFA challenge', () => {
  for (const status of ['success', 'error']) {
    const result = parseSocialCallback(`nyxid://auth/social/callback?status=${status}&access_token=access#mfa_token=private-challenge`);
    assert.equal(result?.status, "error");
    assert.equal(result?.mfaToken, undefined);
  }
});
