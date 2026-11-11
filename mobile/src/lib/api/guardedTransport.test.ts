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
    refresh: async () => { refreshes++; return false; }, invalidate: () => { invalidations++; },
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
      refresh: async () => true, invalidate: () => { invalidations++; },
      requiresAuth: true, retry: true, failure: () => new Error('401'),
    }), /account changed/);
    assert.equal(invalidations, 0);
  }
});

test('MFA refusal retains the session when invalidation is disabled; default 401 invalidates', async () => {
  for (const invalidateOnAuthFailure of [false, undefined]) {
    let invalidations = 0;
    let guardChecks = 0;
    await assert.rejects(guardedJsonTransport({
      send: async () => new Response('{"error":"invalid_mfa_code"}', { status: 401 }),
      read: (response) => response.json(),
      guard: () => { guardChecks++; },
      refresh: async () => false,
      invalidate: () => { invalidations++; },
      requiresAuth: true, retry: false, invalidateOnAuthFailure,
      failure: () => new Error('invalid_mfa_code'),
    }), /invalid_mfa_code/);
    assert.equal(invalidations, invalidateOnAuthFailure === false ? 0 : 1);
    assert.ok(guardChecks >= 4);
  }
});
test('disabling invalidation never admits a successful body from a replacement account', async () => {
  let actor = 'original';
  await assert.rejects(guardedJsonTransport({
    send: async () => new Response('{}'),
    read: async (response) => { actor = 'replacement'; return response.json(); },
    guard: () => { if (actor !== 'original') throw new Error('account changed'); },
    refresh: async () => false, invalidate: () => { throw new Error('must not invalidate'); },
    requiresAuth: true, retry: false, invalidateOnAuthFailure: false,
    failure: () => new Error('request failed'),
  }), /account changed/);
});
