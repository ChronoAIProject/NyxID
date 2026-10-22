import assert from 'node:assert/strict';
import { test } from 'node:test';
import { unregisterCapturedDevice } from './capturedCleanup';
test('captured cleanup sends exactly the invalidated login and never refreshes a rejection', async () => {
  const sent: Array<{url: string; authorization: string}> = [];
  await assert.rejects(unregisterCapturedDevice('http://example.invalid/notifications/devices/current',
    { token: 'device' }, 'invalidated-access', Date.now() + 60000, async (url, init) => {
      sent.push({url: String(url), authorization: new Headers(init?.headers).get('Authorization')!});
      return new Response('{}', {status:401});
    }), /push_unregister_failed/);
  assert.deepEqual(sent, [{url:'http://example.invalid/notifications/devices/current', authorization:'Bearer invalidated-access'}]);
});
test('expired or unparseable captured access skips remote cleanup', async () => {
  let calls = 0;
  for (const expiry of [undefined, Date.now() - 1]) await unregisterCapturedDevice('http://example.invalid', {}, 'old', expiry,
    async () => {calls++; return new Response('{}');});
  assert.equal(calls, 0);
});
