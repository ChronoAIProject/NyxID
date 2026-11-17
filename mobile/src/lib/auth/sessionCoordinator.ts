import { decodeJwtSub } from './jwt';
export type StoredAuthSession = { accessToken: string; refreshToken?: string; accessTokenExpiresAt?: number; userId?: string; sessionGeneration?: number };
export type SessionStorage = { getItemAsync(key: string): Promise<string | null>; setItemAsync(key: string, value: string): Promise<void>; deleteItemAsync(key: string): Promise<void> };
/** Fresh key => SecureStore SecItemAdd applies AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY.
 * One atomic value publishes the entire session: access is never observable
 * beside another login's refresh token. Android uses the same upgrade protocol
 * to retain existing encrypted preferences; accessibility is iOS-only. */
export const SESSION_RECORD_KEY = 'nyxid.auth.v2.session';
const LEGACY_KEYS = ['access_token', 'refresh_token', 'access_token_expires_at', 'user_id', 'storage_format'].map(key => `nyxid.auth.${key}`);
type RecordValue = { version: 2; session: Omit<StoredAuthSession, 'sessionGeneration'> | null };
export function createSessionCoordinator(storage: SessionStorage, bind: (actor: string | null) => void) {
  let generation = 0;
  let legacyCleanupComplete = false;
  let queue: Promise<unknown> = Promise.resolve();
  function serial<T>(operation: () => Promise<T>): Promise<T> {
    const result = queue.catch(() => {}).then(operation);
    queue = result.catch(() => {});
    return result;
  }
  function normalize(session: StoredAuthSession): Omit<StoredAuthSession, 'sessionGeneration'> {
    return {
      refreshToken: session.refreshToken,
      accessTokenExpiresAt: Number.isFinite(session.accessTokenExpiresAt) ? session.accessTokenExpiresAt : undefined,
      userId: decodeJwtSub(session.accessToken) ?? session.userId,
      accessToken: session.accessToken,
    };
  }
  async function write(session: StoredAuthSession | null) {
    const value: RecordValue = { version: 2, session: session ? normalize(session) : null };
    await storage.setItemAsync(SESSION_RECORD_KEY, JSON.stringify(value));
  }
  async function removeLegacy() {
    if (legacyCleanupComplete) return;
    // Try every deletion; retain the atomic record if any deletion fails.
    const results = await Promise.allSettled(LEGACY_KEYS.map(key => storage.deleteItemAsync(key)));
    const failed = results.find(result => result.status === 'rejected');
    if (failed?.status === 'rejected') throw failed.reason;
    legacyCleanupComplete = true;
  }
  async function read(expectedGeneration = generation): Promise<StoredAuthSession | null> {
    const record = await storage.getItemAsync(SESSION_RECORD_KEY);
    if (record !== null) {
      const value = JSON.parse(record) as RecordValue;
      if (value.version !== 2 || (value.session !== null && (!value.session || typeof value.session.accessToken !== 'string' || !value.session.accessToken)))
        throw new Error('auth_session_storage_invalid');
      if (generation === expectedGeneration) await removeLegacy().catch(() => {});
      return value.session ? { ...normalize(value.session), sessionGeneration: expectedGeneration } : null;
    }
    const [accessToken, refreshToken, expires, userId] = await Promise.all(LEGACY_KEYS.slice(0,4).map(key => storage.getItemAsync(key)));
    if (!accessToken) {
      if (generation === expectedGeneration && (refreshToken || expires || userId)) await removeLegacy();
      return null;
    }
    const accessTokenExpiresAt = expires ? Number(expires) : NaN;
    const session: StoredAuthSession = { accessToken, refreshToken: refreshToken ?? undefined,
      accessTokenExpiresAt: Number.isFinite(accessTokenExpiresAt) ? accessTokenExpiresAt : undefined,
      userId: decodeJwtSub(accessToken) ?? userId ?? undefined, sessionGeneration: expectedGeneration };
    if (generation === expectedGeneration) {
      // Keep legacy credentials through every failed write. Once the complete
      // record is durable, interruptions during cleanup cannot lose a session.
      try { await write(session); await removeLegacy(); } catch { /* Retry next load. */ }
    }
    return session;
  }
  return {
    generation: () => generation,
    load() {
      const expected = generation;
      return serial(async () => {
        const value = await read(expected);
        return generation === expected ? value : null;
      });
    },
    persist(session: StoredAuthSession) {
      const expected = ++generation; bind(null);
      return serial(async () => {
        if (generation !== expected) throw new Error('auth_session_changed');
        await write(session);
        // A cleanup failure cannot turn a durable login into a failed login.
        await removeLegacy().catch(() => {});
        if (generation !== expected) throw new Error('auth_session_changed');
        bind(decodeJwtSub(session.accessToken) ?? null);
      });
    },
    clear() {
      const expected = ++generation; bind(null);
      return serial(async () => {
        if (generation !== expected) return;
        // Tombstone first prevents interrupted cleanup resurrecting legacy auth.
        await write(null);
        legacyCleanupComplete = false;
        await removeLegacy();
        await storage.deleteItemAsync(SESSION_RECORD_KEY);
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
