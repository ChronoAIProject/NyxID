import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { guardedJsonTransport } from './guardedTransport';

test('real HTTP late 401 cannot refresh or invalidate a replacement account', async () => {
  let release!: () => void;
  let arrived!: () => void;
  const requestArrived = new Promise<void>((resolve) => { arrived = resolve; });
  const server = createServer((_req, res) => {
    release = () => { res.writeHead(401, { 'content-type': 'application/json' }); res.end('{}'); };
    arrived();
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  let actor = 'a', refreshes = 0, invalidations = 0;
  const pending = guardedJsonTransport({
    send: () => fetch(`http://127.0.0.1:${address.port}`), read: (r) => r.json(),
    guard: () => { if (actor !== 'a') throw new Error('account changed'); },
    refresh: async () => { refreshes++; return { kind: 'rejected', status: 401 }; }, invalidate: () => { invalidations++; },
    requiresAuth: true, retry: true, failure: () => new Error('401'),
  });
  try {
    await requestArrived; actor = 'b'; release();
    await assert.rejects(pending, /account changed/);
    assert.equal(refreshes, 0); assert.equal(invalidations, 0);
  } finally { await new Promise<void>((resolve) => server.close(() => resolve())); }
});
test('late parsed body and retry response are guarded before session effects', async () => {
  for (const boundary of ['body', 'retry']) {
    let actor = 'a', sends = 0, invalidations = 0;
    await assert.rejects(guardedJsonTransport({
      send: async () => { sends++; if (boundary === 'retry' && sends === 2) actor = 'b'; return new Response('{}', { status: 401 }); },
      read: async (r) => { if (boundary === 'body') actor = 'b'; return r.json(); },
      guard: () => { if (actor !== 'a') throw new Error('account changed'); },
      refresh: async () => ({ kind: 'refreshed', credentials: {access_token: 'a', refresh_token: 'r'} }), invalidate: () => { invalidations++; },
      requiresAuth: true, retry: true, failure: () => new Error('401'),
    }), /account changed/);
    assert.equal(invalidations, 0);
  }
});
for (const kind of ['transient', 'persistence_failed'] as const) {
  test(`${kind} preserves session and surfaces a retryable error without request replay`, async () => {
    let sends = 0, invalidations = 0;
    await assert.rejects(guardedJsonTransport({
      send: async () => { sends++; return new Response('{}', {status: 401}); }, read: (r) => r.json(), guard: () => {},
      refresh: async () => ({kind}), invalidate: () => { invalidations++; }, requiresAuth: true, retry: true,
      failure: () => new Error('401'),
    }), (error: any) => error.retryable === true);
    assert.equal(sends, 1); assert.equal(invalidations, 0);
  });
}
test('definitive refresh rejection invalidates the failing session', async () => {
  let invalidations = 0;
  await assert.rejects(guardedJsonTransport({ send: async () => new Response('{}', {status: 401}), read: (r) => r.json(),
    guard: () => {}, refresh: async () => ({kind: 'rejected', status: 401}), invalidate: () => { invalidations++; },
    requiresAuth: true, retry: true, failure: () => new Error('401') }), /401/);
  assert.equal(invalidations, 1);
});
