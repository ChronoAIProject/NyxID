import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createServer } from 'node:http';
import { createSessionCoordinator, SESSION_RECORD_KEY, type SessionStorage } from './sessionCoordinator';
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
      request: async () => (await fetch(`http://127.0.0.1:${address.port}`)).json(), expiresAt: (seconds) => Date.now() + seconds * 1000 });
    try {
      const pending = refresh(); await requested;
      if (replacement === 'b') await coordinator.persist(b); else await coordinator.clear();
      release(); await assert.rejects(pending, /auth_session_changed/);
      assert.equal((await coordinator.load())?.accessToken ?? null, replacement === 'b' ? b.accessToken : null);
      assert.equal(actor(), replacement === 'b' ? 'b' : null);
    } finally { await new Promise<void>((resolve) => server.close(() => resolve())); }
  }
});
test('single-flight is shared within a login and isolated across account switch', { timeout: 10000 }, async () => {
  const { coordinator, actor } = setup(); await coordinator.persist(a);
  const pending: Array<{ token: string; release: (value: any) => void }> = [];
  const refresh = createSessionRefresh({ load: coordinator.load, commit: coordinator.commitRefresh,
    request: (token) => new Promise((release) => { pending.push({ token, release }); }), expiresAt: (v) => v });
  const first = refresh(), shared = refresh();
  const firstRejected = assert.rejects(first, /auth_session_changed/), sharedRejected = assert.rejects(shared, /auth_session_changed/);
  while (pending.length < 1) await new Promise((resolve) => setImmediate(resolve));
  assert.equal(pending.length, 1);
  await coordinator.persist(b); const successor = refresh();
  while (pending.length < 2) await new Promise((resolve) => setImmediate(resolve));
  pending[0]!.release({ access_token: token('a','rotated'), refresh_token:'ra', expires_in:900 });
  await firstRejected; await sharedRejected;
  pending[1]!.release({ access_token: token('b','rotated'), refresh_token:'rb', expires_in:900 });
  assert.ok(await successor); assert.equal(actor(), 'b');
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
  assert.equal(JSON.parse(values.get(SESSION_RECORD_KEY)!).session.refreshToken, b.refreshToken); assert.equal(actor(), 'b');
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

test(
  'rotation atomically publishes successor credentials and a failed write never deletes the old record',
  { timeout: 10000 },
  async () => {
    const { coordinator, storage, values } = setup();
    await coordinator.persist(a);
    const loaded = (await coordinator.load())!;
    const deletes: string[] = [];
    const originalDelete = storage.deleteItemAsync,
      originalSet = storage.setItemAsync;
    storage.deleteItemAsync = async (key) => {
      deletes.push(key);
      return originalDelete(key);
    };
    // The process dies (or the keychain refuses) while the access token is written.
    storage.setItemAsync = async (key, value) => {
      if (key === SESSION_RECORD_KEY) throw new Error('interrupted');
      return originalSet(key, value);
    };
    const rotated = {
      accessToken: token('a', 'rotated'),
      refreshToken: 'rotated-a',
      accessTokenExpiresAt: 1,
    };
    assert.equal(await coordinator.commitRefresh(loaded, rotated), true);
    assert.equal(JSON.parse(values.get(SESSION_RECORD_KEY)!).session.refreshToken, a.refreshToken);
    storage.setItemAsync = originalSet;
    assert.deepEqual(
      deletes.filter(
        (key) =>
          key !== 'nyxid.auth.user_id' &&
          key !== 'nyxid.auth.access_token_expires_at',
      ),
      [],
    );
    const after = await coordinator.load();
    assert.equal(after?.accessToken, rotated.accessToken);
    assert.equal(after?.refreshToken, rotated.refreshToken);
    assert.equal(JSON.parse(values.get(SESSION_RECORD_KEY)!).session.refreshToken, rotated.refreshToken);
  },
);

test(
  'a failed rotation write keeps the successor in memory until a later load persists it',
  { timeout: 10000 },
  async () => {
    const { coordinator, storage } = setup();
    await coordinator.persist(a);
    const loaded = (await coordinator.load())!;
    const originalSet = storage.setItemAsync;
    storage.setItemAsync = async () => {
      throw new Error('keychain_unavailable');
    };
    assert.equal(await coordinator.commitRefresh(loaded, {
      accessToken: token('a', 'rotated'), refreshToken: 'rotated-a',
    }), true);
    assert.equal((await coordinator.load())?.refreshToken, 'rotated-a');
    storage.setItemAsync = originalSet;
    const after = await coordinator.load();
    assert.equal(after?.accessToken, token('a', 'rotated'));
    assert.equal(after?.refreshToken, 'rotated-a');
  },
);

test(
  'a session from an older build is copied once to the fresh atomic record before legacy removal',
  { timeout: 10000 },
  async () => {
    const { coordinator, storage, values } = setup();
    // Written by a previous build: no format marker.
    values.set('nyxid.auth.access_token', a.accessToken);
    values.set('nyxid.auth.refresh_token', a.refreshToken);
    values.set('nyxid.auth.user_id', 'a');
    const readded: string[] = [];
    const originalDelete = storage.deleteItemAsync;
    storage.deleteItemAsync = async (key) => {
      readded.push(key);
      return originalDelete(key);
    };
    assert.equal((await coordinator.load())?.refreshToken, a.refreshToken);
    assert.deepEqual(readded, [
      'nyxid.auth.access_token',
      'nyxid.auth.refresh_token',
      'nyxid.auth.access_token_expires_at',
      'nyxid.auth.user_id',
      'nyxid.auth.storage_format',
    ]);
    const durable = JSON.parse(values.get(SESSION_RECORD_KEY)!);
    assert.equal(durable.version, 2);
    assert.equal(durable.session.accessToken, a.accessToken);
    assert.equal(durable.session.refreshToken, a.refreshToken);
    assert.equal(values.has('nyxid.auth.access_token'), false);
    readded.length = 0;
    assert.equal((await coordinator.load())?.accessToken, a.accessToken);
    assert.deepEqual(readded, []);
  },
);

test(
  'a failed migration still returns the stored session',
  { timeout: 10000 },
  async () => {
    const { coordinator, storage, values } = setup();
    values.set('nyxid.auth.access_token', a.accessToken);
    values.set('nyxid.auth.refresh_token', a.refreshToken);
    storage.setItemAsync = async () => { throw new Error('keychain_unavailable'); };
    assert.equal((await coordinator.load())?.refreshToken, a.refreshToken);
    assert.equal(values.get('nyxid.auth.refresh_token'), a.refreshToken);
  },
);

test(
  'an unreadable keychain rejects the load and leaves every stored key in place',
  { timeout: 10000 },
  async () => {
    const { coordinator, storage, values } = setup();
    await coordinator.persist(a);
    const snapshot = new Map(values);
    const originalGet = storage.getItemAsync;
    storage.getItemAsync = async () => {
      throw new Error('User interaction is not allowed.');
    };
    await assert.rejects(coordinator.load(), /not allowed/);
    storage.getItemAsync = originalGet;
    assert.deepEqual(values, snapshot);
    assert.equal((await coordinator.load())?.accessToken, a.accessToken);
  },
);

test('P1-3 a refused refresh from a replaced login is a stale operation, never a refusal of the successor', async () => {
  for (const replacement of ['login', 'logout']) {
    const { coordinator } = setup(); await coordinator.persist(a);
    let release!: (value: null) => void;
    const refresh = createSessionRefresh({ load: coordinator.load, commit: coordinator.commitRefresh,
      request: () => new Promise<null>(resolve => { release = resolve; }), expiresAt: v => v });
    const pending = refresh(); const rejected = assert.rejects(pending, /auth_session_changed/);
    while (!release) await new Promise(resolve => setImmediate(resolve));
    if (replacement === 'login') await coordinator.persist(b); else await coordinator.clear();
    release(null); await rejected;
    assert.equal((await coordinator.load())?.refreshToken ?? null, replacement === 'login' ? b.refreshToken : null);
  }
});
