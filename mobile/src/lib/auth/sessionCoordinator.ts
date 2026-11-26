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
  // Only failed clears may extend this generation interval. An intent fences
  // immediately; failure restores the prior floor, success keeps the fence.
  let refreshLineageStart = 0;
  function ownsRefreshLineage(expected: StoredAuthSession) {
    return expected.sessionGeneration !== undefined && expected.sessionGeneration >= refreshLineageStart
      && expected.sessionGeneration <= generation;
  }
  let legacyCleanupComplete = false;
  let known: { generation: number; session: StoredAuthSession | null } | undefined;
  function remember(session: StoredAuthSession | null, expected: number) {
    if (generation === expected) known = { generation: expected, session };
    return session;
  }
  let pendingSuccessor: { generation: number; session: StoredAuthSession } | null = null;
  async function persistSuccessor(candidate: NonNullable<typeof pendingSuccessor>, attempts: number) {
    // The server has already retired the predecessor. Publish memory first and
    // bound native retries; later loads keep trying the durable write.
    for (let attempt = 0; attempt < attempts; attempt++) {
      if (generation !== candidate.generation || pendingSuccessor !== candidate) return;
      try {
        await write(candidate.session);
        if (pendingSuccessor === candidate) pendingSuccessor = null;
        return;
      } catch { /* Keep the successor, never reuse the retired token. */ }
    }
  }
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
    const candidate = pendingSuccessor;
    if (candidate?.generation === expectedGeneration && generation === expectedGeneration) {
      await persistSuccessor(candidate, 1);
      return { ...candidate.session, sessionGeneration: expectedGeneration };
    }
    const record = await storage.getItemAsync(SESSION_RECORD_KEY);
    if (record !== null) {
      const value = JSON.parse(record) as RecordValue;
      if (value.version !== 2 || (value.session !== null && (!value.session || typeof value.session.accessToken !== 'string' || !value.session.accessToken)))
        throw new Error('auth_session_storage_invalid');
      if (generation === expectedGeneration) await removeLegacy().catch(() => {});
      return remember(value.session ? { ...normalize(value.session), sessionGeneration: expectedGeneration } : null, expectedGeneration);
    }
    const [accessToken, refreshToken, expires, userId] = await Promise.all(LEGACY_KEYS.slice(0,4).map(key => storage.getItemAsync(key)));
    if (!accessToken) {
      if (generation === expectedGeneration && (refreshToken || expires || userId)) await removeLegacy();
      return remember(null, expectedGeneration);
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
    return remember(session, expectedGeneration);
  }
  return {
    generation: () => generation,
    // Best-effort teardown needs the latest admitted pair without native I/O.
    peek(): StoredAuthSession | null {
      const session = pendingSuccessor?.generation === generation ? pendingSuccessor.session
        : known?.generation === generation ? known.session : null;
      return session ? { ...session, sessionGeneration: generation } : null;
    },
    load() {
      const expected = generation;
      return serial(async () => {
        const value = await read(expected);
        return generation === expected ? value : null;
      });
    },
    persist(session: StoredAuthSession) {
      refreshLineageStart = generation + 1;
      const expected = ++generation; pendingSuccessor = null; known = undefined; bind(null);
      return serial(async () => {
        if (generation !== expected) throw new Error('auth_session_changed');
        await write(session);
        remember({ ...normalize(session), sessionGeneration: expected }, expected);
        // A cleanup failure cannot turn a durable login into a failed login.
        await removeLegacy().catch(() => {});
        if (generation !== expected) throw new Error('auth_session_changed');
        bind(decodeJwtSub(session.accessToken) ?? null);
      });
    },
    clear() {
      const previousGeneration = generation;
      const previousLineageStart = refreshLineageStart;
      refreshLineageStart = generation + 1;
      const previousKnown = known?.generation === previousGeneration ? known : undefined;
      const previousSuccessor = pendingSuccessor?.generation === previousGeneration ? pendingSuccessor : null;
      const expected = ++generation; pendingSuccessor = null; known = undefined; bind(null);
      return serial(async () => {
        if (generation !== expected) return;
        // Tombstone first prevents interrupted cleanup resurrecting legacy auth.
        try {
          await write(null);
        } catch (error) {
          // Failure leaves the admitted session usable, including a successor
          // that exists only in memory. Rebase it into this intent's generation
          // while retaining same-pair refresh lineage only after this clear fails.
          if (generation === expected) {
            const retained = previousSuccessor?.session ?? previousKnown?.session;
            if (retained) {
              refreshLineageStart = previousLineageStart;
              // Android commit() updates native memory before reporting disk
              // failure. The recovery slot must outrank that cached tombstone,
              // even when this pair was already durable before sign-out.
              pendingSuccessor = { generation: expected, session: retained };
              remember({ ...retained, sessionGeneration: expected }, expected);
            } else if (previousKnown) remember(null, expected);
            bind(retained ? decodeJwtSub(retained.accessToken) ?? null : null);
          }
          throw error;
        }
        remember(null, expected);
        legacyCleanupComplete = false;
        // Keep the authoritative nonsecret tombstone even if native deletion
        // silently fails (iOS ignores SecItemDelete status). Legacy deletion is
        // independent housekeeping; a later explicit login overwrites v2.
        void removeLegacy().catch(() => { legacyCleanupComplete = false; });
      });
    },
    commitRefresh(expected: StoredAuthSession, next: StoredAuthSession) {
      return serial(async () => {
        if (!ownsRefreshLineage(expected)) return false;
        // The captured predecessor was loaded by this coordinator. Retain the
        // server's successor before any further fallible native operation.
        const current = known?.generation === generation ? known.session : await read(generation);
        if (!ownsRefreshLineage(expected) || current?.accessToken !== expected.accessToken || current?.refreshToken !== expected.refreshToken
          || decodeJwtSub(next.accessToken) !== decodeJwtSub(expected.accessToken)) return false;
        const candidate = { generation, session: normalize(next) };
        pendingSuccessor = candidate;
        remember({ ...candidate.session, sessionGeneration: generation }, generation);
        await persistSuccessor(candidate, 3);
        if (!ownsRefreshLineage(expected)) return false;
        bind(decodeJwtSub(next.accessToken) ?? null);
        return true;
      });
    },
  };
}
