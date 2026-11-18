import { decodeJwtSub } from './jwt';
export type StoredAuthSession = { accessToken: string; refreshToken?: string; accessTokenExpiresAt?: number; userId?: string; sessionGeneration?: number };
export type SessionStorage = { getItemAsync(key: string): Promise<string | null>; setItemAsync(key: string, value: string): Promise<void>; deleteItemAsync(key: string): Promise<void> };
const ACCESS = 'nyxid.auth.access_token', REFRESH = 'nyxid.auth.refresh_token', EXPIRES = 'nyxid.auth.access_token_expires_at', USER = 'nyxid.auth.user_id';
/** Non-secret marker. Version 2: items use the after-first-unlock keychain class. */
const DRAFT_KEY = 'nyxid.assistant.draft_key';
const FORMAT = 'nyxid.auth.storage_format',
  STORAGE_FORMAT = '2';
/** All SecureStore operations share this queue. Explicit login/logout advances
 * generation synchronously, fencing old network responses before they commit.
 * Refresh commits compare both generation and original access/refresh tokens.
 */
export function createSessionCoordinator(storage: SessionStorage, bind: (actor: string | null) => void) {
  let generation = 0;
  let queue: Promise<unknown> = Promise.resolve();
  function serial<T>(operation: () => Promise<T>): Promise<T> {
    const result = queue.catch(() => {}).then(operation);
    queue = result.catch(() => {});
    return result;
  }
  async function read(expectedGeneration = generation): Promise<StoredAuthSession | null> {
    const [accessToken, refreshToken, expires, userId] = await Promise.all([ACCESS, REFRESH, EXPIRES, USER].map((key) => storage.getItemAsync(key)));
    if (!accessToken) {
      // Clean interrupted legacy writes inside this serialized operation.
      // Never recursively call clear(), which queues behind this read.
      if (generation === expectedGeneration && (refreshToken || expires || userId))
        await Promise.all([REFRESH, EXPIRES, USER].map((key) => storage.deleteItemAsync(key)));
      return null;
    }
    const accessTokenExpiresAt = expires ? Number(expires) : NaN;
    return { accessToken, refreshToken: refreshToken ?? undefined,
      accessTokenExpiresAt: Number.isFinite(accessTokenExpiresAt) ? accessTokenExpiresAt : undefined,
      userId: decodeJwtSub(accessToken) ?? userId ?? undefined, sessionGeneration: generation };
  }
  async function replace(key: string, value: string | undefined) {
    // SecureStore's update path rewrites only the data of an existing item and
    // keeps its original keychain accessibility. Re-adding applies the current
    // class, which migrates items written under the old WHEN_UNLOCKED default.
    await storage.deleteItemAsync(key);
    if (value) await storage.setItemAsync(key, value);
  }
  async function put(key: string, value: string | undefined) {
    if (value) await storage.setItemAsync(key, value);
    else await storage.deleteItemAsync(key);
  }
  function fields(session: StoredAuthSession) {
    return [[REFRESH, session.refreshToken], [EXPIRES, Number.isFinite(session.accessTokenExpiresAt) ? String(session.accessTokenExpiresAt) : undefined], [USER, decodeJwtSub(session.accessToken)]] as const;
  }
  async function writeLogin(session: StoredAuthSession) {
    // Remove the access token first: interrupted multi-key writes cannot leave
    // an apparently authenticated mixed-token record on the next app launch.
    // Every item is re-added, so a login always uses the current storage class.
    await storage.deleteItemAsync(ACCESS);
    await storage.deleteItemAsync(DRAFT_KEY);
    for (const [key, value] of fields(session)) await replace(key, value);
    await storage.setItemAsync(ACCESS, session.accessToken);
    await storage.setItemAsync(FORMAT, STORAGE_FORMAT);
  }
  async function writeRotation(session: StoredAuthSession) {
    // Same subject (verified by the caller). Update in place and store the
    // rotated refresh token before touching the access token: the server has
    // already retired the old refresh token, so a deleted or lost successor
    // would end the session. A failed in-place update keeps the old token,
    // which the server still honours as a retry within its grace window, and
    // an interruption leaves old access + new refresh, which refreshes cleanly.
    for (const [key, value] of fields(session)) await put(key, value);
    await storage.setItemAsync(ACCESS, session.accessToken);
  }
  /** One-time re-add of a session written by an older build, run only after a
   * successful read proves the keychain is currently readable. */
  async function migrate(expectedGeneration: number) {
    if (
      generation !== expectedGeneration ||
      (await storage.getItemAsync(FORMAT)) === STORAGE_FORMAT
    )
      return;
    const values = await Promise.all(
      [REFRESH, EXPIRES, USER, ACCESS].map((key) => storage.getItemAsync(key)),
    );
    if (generation !== expectedGeneration || !values[3]) return;
    // Same account's values are re-added in place, access last. Each re-add is
    // a delete + add of one key, so only a crash inside that pair can lose a
    // key, once per install, versus every locked background launch before.
    const keys = [REFRESH, EXPIRES, USER, ACCESS];
    for (let i = 0; i < keys.length; i += 1)
      if (values[i]) await replace(keys[i]!, values[i]!);
    await storage.setItemAsync(FORMAT, STORAGE_FORMAT);
  }
  return {
    generation: () => generation,
    load() {
      const expected = generation;
      return serial(async () => { const value = await read(expected);
        if (generation !== expected) return null;
        // Migration is best effort: the session already loaded, and a later
        // load retries. Never turn a migration failure into a logout.
        if (value) await migrate(expected).catch(() => {});
        return value; });
    },
    draftKey(expected: number, create?: () => Promise<string>): Promise<string | null> {
      return serial(async () => {
        if (generation !== expected) return null;
        let key = await storage.getItemAsync(DRAFT_KEY);
        if (!key && create && generation === expected) {
          key = await create();
          if (generation !== expected) return null;
          await storage.setItemAsync(DRAFT_KEY, key);
        }
        return generation === expected ? key : null;
      });
    },
    persist(session: StoredAuthSession) {
      const expected = ++generation; bind(null);
      return serial(async () => {
        if (generation !== expected) throw new Error('auth_session_changed');
        await writeLogin(session);
        if (generation !== expected) throw new Error('auth_session_changed');
        bind(decodeJwtSub(session.accessToken) ?? null);
      });
    },
    clear() {
      const expected = ++generation; bind(null);
      return serial(async () => {
        if (generation !== expected) return;
        await Promise.all([ACCESS, REFRESH, EXPIRES, USER, DRAFT_KEY].map((key) => storage.deleteItemAsync(key)));
      });
    },
    commitRefresh(expected: StoredAuthSession, next: StoredAuthSession) {
      return serial(async () => {
        if (generation !== expected.sessionGeneration) return false;
        const current = await read(expected.sessionGeneration);
        if (generation !== expected.sessionGeneration || current?.accessToken !== expected.accessToken || current?.refreshToken !== expected.refreshToken
          || decodeJwtSub(next.accessToken) !== decodeJwtSub(expected.accessToken)) return false;
        await writeRotation(next);
        if (generation !== expected.sessionGeneration) return false;
        bind(decodeJwtSub(next.accessToken) ?? null);
        return true;
      });
    },
  };
}
