import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mfaPrompt } from './mfaPrompt';

test('social MFA hides password fields and identifies each provider', () => {
  for (const [provider, name] of [['google', 'Google'], ['github', 'GitHub'], ['apple', 'Apple']]) {
    const prompt = mfaPrompt(provider!, false);
    assert.equal(prompt.showPasswordFields, false);
    assert.equal(prompt.title, `Verify your ${name} sign-in`);
    assert.equal(prompt.label, 'Authenticator code');
    assert.equal(prompt.placeholder, '000000');
    assert.equal(prompt.toggle, 'Use a recovery code');
    const recovery = mfaPrompt(provider!, true);
    assert.equal(recovery.showPasswordFields, false);
    assert.equal(recovery.label, 'Recovery code');
    assert.equal(recovery.placeholder, 'Paste a recovery code');
    assert.equal(recovery.toggle, 'Use an authenticator code');
  }
  assert.equal(mfaPrompt(null, false).showPasswordFields, true);
});
