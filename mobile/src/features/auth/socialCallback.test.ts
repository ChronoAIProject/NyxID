import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parseSocialCallback } from './socialCallback';

test('Google, GitHub and Apple MFA callbacks carry only a fragment challenge', () => {
  for (const provider of ['google', 'github', 'apple']) {
    const result = parseSocialCallback(`nyxid://auth/social/callback#status=mfa_required&provider=${provider}&mfa_token=private-challenge`);
    assert.deepEqual(result, { status: 'mfa_required', provider, mfaToken: 'private-challenge' });
    assert.equal(result?.accessToken, undefined);
    assert.equal(result?.refreshToken, undefined);
    assert.equal(parseSocialCallback(`nyxid://auth/social/callback?status=mfa_required&provider=${provider}&mfa_token=query-token`)?.mfaToken, undefined);
  }
});

test('social callback parser rejects foreign origins and prefix lookalikes', () => {
  for (const url of ['https://auth/social/callback#status=mfa_required', 'nyxid://auth/social/callback-extra#status=mfa_required', 'nyxid://auth.evil/social/callback#status=mfa_required', 'nyxid://other/social/callback#status=mfa_required']) {
    assert.equal(parseSocialCallback(url), null);
  }
});

test('non-MFA social login preserves its token response contract', () => {
  assert.deepEqual(parseSocialCallback('nyxid://auth/social/callback?status=success&provider=apple&access_token=access&refresh_token=refresh&expires_in=900'), {
    status: 'success', provider: 'apple', accessToken: 'access', refreshToken: 'refresh', expiresIn: 900,
  });
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
    assert.equal(result?.status, status);
    assert.equal(result?.mfaToken, undefined);
  }
});
