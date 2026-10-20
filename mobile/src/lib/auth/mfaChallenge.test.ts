import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createServer } from 'node:http';
import { MfaChallengeError, mfaChallengeFromResponse } from './mfaChallenge';

test('mobile continuation submits legacy/new recovery or TOTP with its own primary-factor challenge', { timeout: 10000 }, async () => {
  const bodies: any[] = [];
  const server = createServer(async (req,res) => {
    let body = ''; for await (const chunk of req) body += chunk;
    bodies.push(JSON.parse(body));
    res.writeHead(200, {'content-type':'application/json'}); res.end(JSON.stringify({user_id:'a',access_token:'completed'}));
  });
  await new Promise<void>((resolve) => server.listen(0,'127.0.0.1',resolve));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const challenge = new MfaChallengeError('distinctive-first-factor-token');
  assert.ok(!JSON.stringify(challenge).includes('distinctive-first-factor-token'));
  try {
    const request = async (body: any) => (await fetch(`http://127.0.0.1:${address.port}/auth/mfa/verify`,{method:'POST',body:JSON.stringify({...body,client:'mobile'})})).json();
    for (const code of ['123456','Ab_cd-12','0123456789abcdef0123456789abcdef']) {
      assert.equal((await challenge.verify(code,request)).user_id,'a');
      assert.deepEqual(bodies.at(-1),{code,mfa_token:'distinctive-first-factor-token',client:'mobile'});
    }
    for (const code of ['12345','1234567','x'.repeat(32),'z'.repeat(500)]) await assert.rejects(challenge.verify(code,request));
    assert.equal(bodies.length,3);
  } finally { await new Promise<void>((resolve) => server.close(() => resolve())); }
});

// Exercise the same outcome dispatcher used by AuthHome, including rejected
// first-factor responses: neither outcome may reach state after authority moves.
test('late MFA-required response cannot install a challenge after session switch or unmount', async () => {
  const { runAuthAttempt } = await import('./authAttempt');
  for (const invalidate of ['session', 'unmount']) {
    let generation = 0; let mounted = true;
    let reject!: (error: unknown) => void;
    const response = new Promise<void>((_, fail) => { reject = fail; });
    const outcomes: unknown[] = [];
    const attempt = runAuthAttempt(() => response, () => mounted && generation === 0,
      async (value) => { outcomes.push(value); }, (error) => { outcomes.push(error); });
    if (invalidate === 'session') generation++; else mounted = false;
    reject(new MfaChallengeError('late-private-token'));
    await attempt;
    assert.deepEqual(outcomes, []);
  }
});

test('documented first-factor error becomes a contained challenge; other responses never do', () => {
  const payload = {error_code:2002,session_token:'private-challenge',message:'must not retain raw response'};
  const challenge = mfaChallengeFromResponse(payload,403);
  assert.ok(challenge instanceof MfaChallengeError);
  assert.ok(!JSON.stringify(challenge).includes('private-challenge'));
  assert.ok(!JSON.stringify(challenge).includes('must not retain raw response'));
  for (const [body,status] of [[payload,200],[payload,401],[{...payload,error_code:2003},403],
    [{...payload,session_token:'x'.repeat(129)},403],[{...payload,session_token:'wrong token'},403],[null,403]] as const) {
    assert.equal(mfaChallengeFromResponse(body,status),null);
  }
});
