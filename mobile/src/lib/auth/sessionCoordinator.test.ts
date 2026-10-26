import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createServer } from 'node:http';
import { createSessionCoordinator, type SessionStorage } from './sessionCoordinator';
import { createSessionRefresh } from './sessionRefresh';
const token = (sub: string, label: string) => `e30.${Buffer.from(JSON.stringify({ sub, label })).toString('base64url')}.sig`;
const a = { accessToken: token('a', 'original'), refreshToken: 'refresh-a' };
const b = { accessToken: token('b', 'original'), refreshToken: 'refresh-b' };
function setup() {
  const values = new Map<string, string>(); let actor: string | null = null;
  const storage: SessionStorage = { getItemAsync: async (key) => values.get(key) ?? null,
    setItemAsync: async (key, value) => { values.set(key, value); }, deleteItemAsync: async (key) => { values.delete(key); } };
  return { coordinator: createSessionCoordinator(storage, (value) => { actor = value; }), actor: () => actor, storage, values };
}
test('held HTTP refresh from A cannot overwrite a new login B or logout', { timeout: 10000 }, async () => {
  for (const replacement of ['b', 'logout']) {
    const { coordinator, actor } = setup(); await coordinator.persist(a);
    let arrived!: () => void, release!: () => void;
    const requested = new Promise<void>((resolve) => { arrived = resolve; });
    const server = createServer((_req, res) => { release = () => { res.writeHead(200, { 'content-type': 'application/json' }); res.end(JSON.stringify({ access_token: token('a', 'rotated'), refresh_token: 'rotated-a', expires_in: 900 })); }; arrived(); });
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    const refresh = createSessionRefresh({ load: coordinator.load, commit: coordinator.commitRefresh,
      request: async () => ({ kind: 'refreshed', credentials: await (await fetch(`http://127.0.0.1:${address.port}`)).json() }), expiresAt: (seconds) => Date.now() + seconds * 1000 });
    try {
      const pending = refresh(); await requested;
      if (replacement === 'b') await coordinator.persist(b); else await coordinator.clear();
      release(); assert.equal((await pending).kind, 'superseded');
      assert.equal((await coordinator.load())?.accessToken ?? null, replacement === 'b' ? b.accessToken : null);
      assert.equal(actor(), replacement === 'b' ? 'b' : null);
    } finally { await new Promise<void>((resolve) => server.close(() => resolve())); }
  }
});
test('single-flight is shared within a login and isolated across account switch', { timeout: 10000 }, async () => {
  const { coordinator, actor } = setup(); await coordinator.persist(a);
  const pending: Array<{ token: string; release: (value: any) => void }> = [];
  const refresh = createSessionRefresh({ load: coordinator.load, commit: coordinator.commitRefresh,
    request: async (token) => ({ kind: 'refreshed', credentials: await new Promise<any>((release) => { pending.push({ token, release }); }) }), expiresAt: (v) => v });
  const first = refresh(), shared = refresh();
  while (pending.length < 1) await new Promise((resolve) => setImmediate(resolve));
  assert.equal(pending.length, 1);
  await coordinator.persist(b); const successor = refresh();
  while (pending.length < 2) await new Promise((resolve) => setImmediate(resolve));
  pending[0]!.release({ access_token: token('a','rotated'), refresh_token:'ra', expires_in:900 });
  assert.equal((await first).kind, 'superseded'); assert.equal((await shared).kind, 'superseded');
  pending[1]!.release({ access_token: token('b','rotated'), refresh_token:'rb', expires_in:900 });
  assert.equal((await successor).kind, 'refreshed'); assert.equal(actor(), 'b');
  assert.equal((await coordinator.load())?.refreshToken, 'rb');
});
test('serialized storage mutations never expose mixed successor credentials', { timeout: 10000 }, async () => {
  const { coordinator, storage, values, actor } = setup(); await coordinator.persist(a);
  let release!: () => void; const original = storage.setItemAsync; let hold = true;
  storage.setItemAsync = async (key, value) => {
    if (hold) { hold = false; await new Promise<void>((resolve) => { release = resolve; }); }
    return original(key, value);
  };
  const stale = coordinator.persist({ accessToken: token('a','again'), refreshToken:'a-again' });
  const staleRejected = assert.rejects(stale, /auth_session_changed/);
  while (!release) await new Promise((resolve) => setImmediate(resolve));
  const successor = coordinator.persist(b); const read = coordinator.load(); release();
  await staleRejected; await successor;
  assert.equal((await read)?.accessToken, b.accessToken);
  assert.equal(values.get('nyxid.auth.refresh_token'), b.refreshToken); assert.equal(actor(), 'b');
});

test('missing access cleans orphan secrets in the serialized read without harming a queued successor', { timeout: 10000 }, async () => {
  const { coordinator, values, storage, actor } = setup();
  for (const key of ['refresh_token', 'user_id', 'access_token_expires_at']) values.set(`nyxid.auth.${key}`, 'orphan-secret');
  assert.equal(await coordinator.load(), null);
  assert.equal(values.size, 0);
  values.set('nyxid.auth.refresh_token', 'orphan-secret');
  let release!: () => void;
  const original = storage.getItemAsync;
  storage.getItemAsync = async (key) => {
    if (key.endsWith('access_token')) { await new Promise<void>((resolve) => { release = resolve; }); }
    return original(key);
  };
  const stale = coordinator.load();
  while (!release) await new Promise((resolve) => setImmediate(resolve));
  const successor = coordinator.persist(b); release();
  assert.equal(await stale, null); await successor;
  storage.getItemAsync = original;
  assert.equal((await coordinator.load())?.refreshToken, b.refreshToken); assert.equal(actor(), 'b');
});

for (const failedKey of ['refresh_token', 'access_token', 'user_id']) {
  test(`refresh persistence retries the same pair and retains recoverable credentials at ${failedKey}`, async () => {
    const { coordinator, storage, values } = setup(); await coordinator.persist(a);
    const expected = (await coordinator.load())!;
    const writes: string[] = []; const original = storage.setItemAsync;
    storage.setItemAsync = async (key, value) => {
      writes.push(key);
      if (key === `nyxid.auth.${failedKey}`) throw new Error('disk unavailable');
      await original(key, value);
    };
    const result = await coordinator.commitRefresh(expected, { accessToken: token('a','q'), refreshToken: 'q' });
    assert.equal(result.kind, failedKey === 'user_id' ? 'committed' : 'persistence_failed');
    assert.equal(writes[0], 'nyxid.auth.refresh_token');
    assert.equal(writes.filter((key) => key === `nyxid.auth.${failedKey}`).length, 3);
    const loaded = await coordinator.load(); assert.ok(loaded);
    assert.equal(loaded.accessToken, failedKey === 'user_id' ? token('a','q') : a.accessToken);
    assert.equal(loaded.refreshToken, failedKey === 'refresh_token' ? a.refreshToken : 'q');
    assert.ok(values.has('nyxid.auth.access_token'));
  });
}
for (const operation of ['logout', 'replace']) {
  test(`restart after partial ${operation} cleans orphan credentials and never restores the abandoned account`, async () => {
    const { coordinator, storage, values } = setup(); await coordinator.persist(a);
    if (operation === 'logout') {
      const original = storage.deleteItemAsync;
      storage.deleteItemAsync = async (key) => {
        if (key.endsWith('refresh_token')) throw new Error('storage failure');
        await original(key);
      };
      await assert.rejects(coordinator.clear()); storage.deleteItemAsync = original;
    } else {
      const original = storage.setItemAsync;
      storage.setItemAsync = async (key, value) => {
        if (key.endsWith('access_token')) throw new Error('storage failure');
        await original(key, value);
      };
      await assert.rejects(coordinator.persist(b)); storage.setItemAsync = original;
    }
    let actor: string | null = null;
    const restarted = createSessionCoordinator(storage, (value) => { actor = value; });
    assert.equal(await restarted.load(), null); assert.equal(actor, null); assert.equal(values.size, 0);
    let requests = 0;
    const refresh = createSessionRefresh({ load: restarted.load, commit: restarted.commitRefresh,
      request: async () => { requests++; return {kind: 'transient'}; }, expiresAt: () => undefined });
    assert.equal((await refresh()).kind, 'no_credentials'); assert.equal(requests, 0);
  });
}
test('concurrent callers share one transient outcome without changing stored credentials', async () => {
  const { coordinator } = setup(); await coordinator.persist(a);
  let requests = 0, release!: () => void;
  const refresh = createSessionRefresh({load: coordinator.load, commit: coordinator.commitRefresh,
    request: async () => { requests++; await new Promise<void>((resolve) => { release = resolve; }); return {kind: 'transient'}; }, expiresAt: () => undefined});
  const first = refresh(), second = refresh();
  while (!release) await new Promise((resolve) => setImmediate(resolve));
  release(); const outcomes = await Promise.all([first, second]);
  assert.equal(requests, 1); assert.equal(outcomes[0], outcomes[1]); assert.equal(outcomes[0].kind, 'transient');
  assert.equal((await coordinator.load())?.refreshToken, a.refreshToken);
});
test('late terminal or transient refresh from replaced login reports superseded', async () => {
  for (const kind of ['rejected', 'transient'] as const) {
    const {coordinator} = setup(); await coordinator.persist(a);
    let release!: () => void;
    const refresh = createSessionRefresh({load: coordinator.load, commit: coordinator.commitRefresh,
      expiresAt: (v) => v, request: async () => {
        await new Promise<void>((resolve) => {release = resolve;});
        return kind === 'rejected' ? {kind, status:401} : {kind, cause:'offline'};
      }});
    const pending = refresh(); while (!release) await new Promise((resolve) => setImmediate(resolve));
    await coordinator.persist(b); release();
    const outcome = await pending; assert.equal(outcome.kind, 'superseded');
    assert.equal((await coordinator.load())?.refreshToken, b.refreshToken);
  }
});
test('storage failure while loading a superseding login remains a typed persistence failure', async () => {
  const {coordinator} = setup(); await coordinator.persist(a);
  const session = (await coordinator.load())!;
  let reads = 0;
  const cause = new Error('storage unavailable');
  const refresh = createSessionRefresh({
    load: async () => { if (reads++ === 0) return session; throw cause; },
    commit: async () => ({kind: 'superseded'}), expiresAt: () => undefined,
    request: async () => ({kind: 'refreshed', credentials: {access_token: a.accessToken, refresh_token: 'successor'}}),
  });
  assert.deepEqual(await refresh(), {kind: 'persistence_failed', cause});
});
