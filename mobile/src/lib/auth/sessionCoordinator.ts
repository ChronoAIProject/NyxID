import { decodeJwtSub } from './jwt';
export type StoredAuthSession = { accessToken: string; refreshToken?: string; accessTokenExpiresAt?: number; userId?: string; sessionGeneration?: number };
export type SessionStorage = { getItemAsync(key: string): Promise<string | null>; setItemAsync(key: string, value: string): Promise<void>; deleteItemAsync(key: string): Promise<void> };
const ACCESS = 'nyxid.auth.access_token', REFRESH = 'nyxid.auth.refresh_token', EXPIRES = 'nyxid.auth.access_token_expires_at', USER = 'nyxid.auth.user_id';
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
  async function write(session: StoredAuthSession) {
    // Remove the access token first: interrupted multi-key writes cannot leave
    // an apparently authenticated mixed-token record on the next app launch.
    await storage.deleteItemAsync(ACCESS);
    for (const [key, value] of [[REFRESH, session.refreshToken], [EXPIRES, Number.isFinite(session.accessTokenExpiresAt) ? String(session.accessTokenExpiresAt) : undefined], [USER, decodeJwtSub(session.accessToken)]] as const) {
      if (value) await storage.setItemAsync(key, value); else await storage.deleteItemAsync(key);
    }
    await storage.setItemAsync(ACCESS, session.accessToken);
  }
  return {
    generation: () => generation,
    load() {
      const expected = generation;
      return serial(async () => { const value = await read(expected); return generation === expected ? value : null; });
    },
    persist(session: StoredAuthSession) {
      const expected = ++generation; bind(null);
      return serial(async () => {
        if (generation !== expected) throw new Error('auth_session_changed');
        await write(session);
        if (generation !== expected) throw new Error('auth_session_changed');
        bind(decodeJwtSub(session.accessToken) ?? null);
      });
    },
    clear() {
      const expected = ++generation; bind(null);
      return serial(async () => {
        if (generation !== expected) return;
        await Promise.all([ACCESS, REFRESH, EXPIRES, USER].map((key) => storage.deleteItemAsync(key)));
      });
    },
    commitRefresh(expected: StoredAuthSession, next: StoredAuthSession) {
      return serial(async () => {
        if (generation !== expected.sessionGeneration) return false;
        const current = await read(expected.sessionGeneration);
        if (generation !== expected.sessionGeneration || current?.accessToken !== expected.accessToken || current?.refreshToken !== expected.refreshToken
          || decodeJwtSub(next.accessToken) !== decodeJwtSub(expected.accessToken)) return false;
        await write(next);
        if (generation !== expected.sessionGeneration) return false;
        bind(decodeJwtSub(next.accessToken) ?? null);
        return true;
      });
    },
  };
}
