import assert from "node:assert/strict";
import test from "node:test";
import {
  createSessionPersistence,
  sessionStorageKeys,
  type SessionKeyValueStore,
} from "./sessionPersistence";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function fixture() {
  let epoch = 0;
  const values = new Map<string, string>();
  const store: SessionKeyValueStore = {
    getItem: async (key) => values.get(key) ?? null,
    setItem: async (key, value) => {
      values.set(key, value);
    },
    deleteItem: async (key) => {
      values.delete(key);
    },
  };
  const advance = () => {
    epoch += 1;
  };
  const api = createSessionPersistence({
    store,
    getAuthEpoch: () => epoch,
    advanceAuthEpoch: advance,
    decodeSubject: (token) => token.split("-")[0],
  });
  return { api, store, values, epoch: () => epoch, advance };
}

const alice = {
  accessToken: "alice-old",
  refreshToken: "alice-refresh",
  userId: "alice",
  accessTokenExpiresAt: 1000,
};
const refreshedAlice = {
  ...alice,
  accessToken: "alice-new",
  refreshToken: "alice-successor",
  accessTokenExpiresAt: 2000,
};
const bob = {
  accessToken: "bob-login",
  refreshToken: "bob-refresh",
  userId: "bob",
  accessTokenExpiresAt: 3000,
};

test("paused old refresh cannot replace a new login or publish mixed account keys", async () => {
  const f = fixture();
  await f.api.persistAuthSession(alice);
  const entered = deferred();
  const resume = deferred();
  const originalSet = f.store.setItem;
  f.store.setItem = async (key, value) => {
    if (key === sessionStorageKeys.userId && value === "alice") {
      entered.resolve();
      await resume.promise;
    }
    await originalSet(key, value);
  };
  const refresh = f.api.persistRefreshedAuthSession(
    refreshedAlice,
    f.epoch(),
    alice.refreshToken,
  );
  await entered.promise;
  const logout = f.api.clearStoredAuthSession();
  assert.equal(
    f.epoch(),
    1,
    "logout fences immediately without waiting for a native write",
  );
  f.advance();
  const login = f.api.persistAuthSession(bob);
  const read = f.api.loadStoredAuthSession();
  resume.resolve();
  assert.equal(await refresh, false);
  await logout;
  await login;
  assert.deepEqual(await read, bob);
  assert.deepEqual(await f.api.loadStoredAuthSession(), bob);
  assert.equal(f.values.get(sessionStorageKeys.refreshToken), "bob-refresh");
});

test("reads wait for an entire native session write", async () => {
  const f = fixture();
  const entered = deferred();
  const resume = deferred();
  const originalSet = f.store.setItem;
  f.store.setItem = async (key, value) => {
    if (key === sessionStorageKeys.userId) {
      entered.resolve();
      await resume.promise;
    }
    await originalSet(key, value);
  };
  const write = f.api.persistAuthSession(alice);
  await entered.promise;
  let readCompleted = false;
  const read = f.api.loadStoredAuthSession().then((session) => {
    readCompleted = true;
    return session;
  });
  await Promise.resolve();
  assert.equal(readCompleted, false);
  resume.resolve();
  await write;
  assert.deepEqual(await read, alice);
});

test("refresh checks both account epoch and current refresh token within the queue", async () => {
  const f = fixture();
  await f.api.persistAuthSession(alice);
  const originalEpoch = f.epoch();
  assert.equal(
    await f.api.persistRefreshedAuthSession(
      refreshedAlice,
      originalEpoch,
      "wrong-refresh",
    ),
    false,
  );
  f.advance();
  assert.equal(
    await f.api.persistRefreshedAuthSession(
      refreshedAlice,
      originalEpoch,
      alice.refreshToken,
    ),
    false,
  );
  assert.deepEqual(await f.api.loadStoredAuthSession(), alice);
});

test("failed writes clean partial session state and the queue admits the next login", async () => {
  const f = fixture();
  await f.api.persistAuthSession(alice);
  const originalSet = f.store.setItem;
  let fail = true;
  f.store.setItem = async (key, value) => {
    if (key === sessionStorageKeys.refreshToken && fail) {
      fail = false;
      throw new Error("storage unavailable");
    }
    await originalSet(key, value);
  };
  await assert.rejects(
    f.api.persistAuthSession(refreshedAlice),
    /storage unavailable/,
  );
  assert.equal(await f.api.loadStoredAuthSession(), null);
  await f.api.persistAuthSession(bob);
  assert.deepEqual(await f.api.loadStoredAuthSession(), bob);
});

test("orphan cleanup clears every key without recursively queueing its own clear", async () => {
  const f = fixture();
  f.values.set(sessionStorageKeys.userId, "alice");
  f.values.set(sessionStorageKeys.refreshToken, "orphan");
  f.values.set(sessionStorageKeys.expiresAt, "1000");
  assert.equal(await f.api.loadStoredAuthSession(), null);
  assert.equal(f.values.size, 0);
  assert.equal(f.epoch(), 1);
  await f.api.persistAuthSession({
    accessToken: "bob-login",
    accessTokenExpiresAt: 3000.8,
  });
  assert.equal((await f.api.loadStoredAuthSession())?.userId, "bob");
  assert.equal(
    (await f.api.loadStoredAuthSession())?.accessTokenExpiresAt,
    3000,
  );
});

test("a queued stale login is fenced before it can write after logout", async () => {
  const f = fixture();
  const pendingLogin = f.api.persistAuthSession(alice);
  const result = assert.rejects(pendingLogin, {
    name: "AbortError",
    message: "auth_session_changed",
  });
  const clear = f.api.clearStoredAuthSession();
  await result;
  await clear;
  assert.equal(await f.api.loadStoredAuthSession(), null);
});
