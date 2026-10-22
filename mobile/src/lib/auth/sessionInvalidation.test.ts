import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createSessionCoordinator } from './sessionCoordinator';
import { createSessionInvalidationHandler, createSessionInvalidationRegistry } from './sessionInvalidation';
import { guardedJsonTransport } from '../api/guardedTransport';
const jwt = (label: string) => `e30.${Buffer.from(JSON.stringify({sub: 'a', label})).toString('base64url')}.sig`;
const p = {accessToken: jwt('p'), refreshToken: 'p'};
const q = {accessToken: jwt('q'), refreshToken: 'q'};
const b = {accessToken: `e30.${Buffer.from(JSON.stringify({sub: 'b'})).toString('base64url')}.sig`, refreshToken: 'b'};
function fixture() {
  const values = new Map<string, string>(); let authenticated = true, localEffects = 0;
  const coordinator = createSessionCoordinator({getItemAsync: async (key) => values.get(key) ?? null,
    setItemAsync: async (key, value) => {values.set(key,value);}, deleteItemAsync: async (key) => {values.delete(key);}}, () => {});
  const registry = createSessionInvalidationRegistry(coordinator.compareAndClear);
  const register = (remoteCleanup: Parameters<typeof createSessionInvalidationHandler>[0]['remoteCleanup']) => {
    registry.setListener(createSessionInvalidationHandler({compareAndClear: coordinator.compareAndClear,
      cleanupIfInvalidated: coordinator.cleanupIfInvalidated, remoteCleanup,
      localCleanup: async () => {localEffects++;}, signedOut: () => {authenticated = false;}}));
  };
  return {coordinator, registry, register, authenticated: () => authenticated, localEffects: () => localEffects};
}
test('registered listener: refresh wins; stale invalidation leaves successor and authenticated UI', async () => {
  const f = fixture(); await f.coordinator.persist(p); const expected = (await f.coordinator.load())!;
  let remote = 0; f.register(async () => {remote++; return true;});
  assert.equal((await f.coordinator.commitRefresh(expected,q)).kind, 'committed');
  await f.registry.invalidate(expected);
  assert.equal((await f.coordinator.load())?.refreshToken,'q'); assert.equal(remote,0);
  assert.equal(f.authenticated(),true); assert.equal(f.localEffects(),0);
});
test('registered listener: invalidation wins; pending refresh cannot restore during push cleanup', async () => {
  const f = fixture(); await f.coordinator.persist(p); const expected = (await f.coordinator.load())!;
  let release!: () => void;
  f.register(async (captured) => {assert.equal(captured.accessToken,p.accessToken); await new Promise<void>((resolve) => {release=resolve;}); return true;});
  const invalidating = f.registry.invalidate(expected);
  while (!release) await new Promise((resolve) => setImmediate(resolve));
  assert.equal(f.coordinator.generation(),expected.sessionGeneration!+1);
  assert.equal((await f.coordinator.commitRefresh(expected,q)).kind,'superseded');
  release(); await invalidating;
  assert.equal(await f.coordinator.load(),null); assert.equal(f.authenticated(),false);
});
for (const pause of ['before_remote','after_remote']) {
  test(`registered listener: new login during ${pause} preserves new push state and UI; cleanup uses captured login`, async () => {
    const f = fixture(); await f.coordinator.persist(p); const expected = (await f.coordinator.load())!;
    let release!: () => void; const sent: string[] = [];
    f.register(async (captured) => {
      if (pause === 'before_remote') await new Promise<void>((resolve) => {release=resolve;});
      sent.push(captured.accessToken);
      if (pause === 'after_remote') await new Promise<void>((resolve) => {release=resolve;});
      return true;
    });
    const invalidating = f.registry.invalidate(expected);
    while (!release) await new Promise((resolve) => setImmediate(resolve));
    await f.coordinator.persist(b); release(); await invalidating;
    assert.deepEqual(sent,[p.accessToken]); assert.equal((await f.coordinator.load())?.refreshToken,'b');
    assert.equal(f.authenticated(),true); assert.equal(f.localEffects(),0);
  });
}
test('delayed retry 401 after a newer refresh in the same login cannot invalidate it', async () => {
  const f = fixture(); await f.coordinator.persist(p); let expected = (await f.coordinator.load())!;
  f.register(async () => true); let sends = 0;
  await assert.rejects(guardedJsonTransport({
    send: async () => {sends++; if (sends === 2) await f.coordinator.commitRefresh(expected,q); return new Response('{}',{status:401});},
    read: (r) => r.json(), guard: () => {},
    refresh: async () => {const next={accessToken:jwt('intermediate'),refreshToken:'intermediate'};
      await f.coordinator.commitRefresh(expected,next); expected=(await f.coordinator.load())!;
      return {kind:'refreshed',credentials:{access_token:next.accessToken,refresh_token:next.refreshToken}};},
    invalidate: () => f.registry.invalidate(expected), requiresAuth:true,retry:true,failure:()=>new Error('401'),
  }),/401/);
  assert.equal((await f.coordinator.load())?.refreshToken,'q'); assert.equal(f.authenticated(),true);
});
