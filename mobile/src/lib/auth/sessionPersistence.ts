export const sessionStorageKeys = {
  accessToken: "nyxid.auth.access_token",
  refreshToken: "nyxid.auth.refresh_token",
  expiresAt: "nyxid.auth.access_token_expires_at",
  userId: "nyxid.auth.user_id",
} as const;

export type StoredAuthSession = {
  accessToken: string;
  refreshToken?: string;
  accessTokenExpiresAt?: number;
  userId?: string;
};

export type SessionKeyValueStore = {
  getItem: (key: string) => Promise<string | null>;
  setItem: (key: string, value: string) => Promise<void>;
  deleteItem: (key: string) => Promise<void>;
};

function sessionChanged(): Error {
  return Object.assign(new Error("auth_session_changed"), {
    name: "AbortError",
  });
}

/** Keep each multi-key session operation together. No read may observe the
 * intermediate keys of a refresh, login, or logout. Failed operations leave
 * the queue usable and never expose a partially written session. */
export function createSessionPersistence({
  store,
  getAuthEpoch,
  advanceAuthEpoch,
  decodeSubject,
}: {
  store: SessionKeyValueStore;
  getAuthEpoch: () => number;
  advanceAuthEpoch: () => void;
  decodeSubject: (token: string) => string | undefined;
}) {
  let queue: Promise<unknown> = Promise.resolve();
  function serialize<T>(operation: () => Promise<T>): Promise<T> {
    const result = queue.then(operation);
    queue = result.catch(() => undefined);
    return result;
  }

  async function rawClear(): Promise<void> {
    // Attempt every deletion even when one key operation fails.
    const results = await Promise.allSettled(
      Object.values(sessionStorageKeys).map((key) => store.deleteItem(key)),
    );
    const failed = results.find((result) => result.status === "rejected");
    if (failed?.status === "rejected") throw failed.reason;
  }

  async function rawLoad(): Promise<StoredAuthSession | null> {
    const [accessToken, refreshToken, expiry, storedUserId] = await Promise.all(
      [
        store.getItem(sessionStorageKeys.accessToken),
        store.getItem(sessionStorageKeys.refreshToken),
        store.getItem(sessionStorageKeys.expiresAt),
        store.getItem(sessionStorageKeys.userId),
      ],
    );
    if (!accessToken) {
      if (refreshToken || expiry || storedUserId) {
        advanceAuthEpoch();
        // This is already inside the queue. Calling the public clear method
        // here would wait for itself and deadlock orphan cleanup.
        await rawClear();
      }
      return null;
    }
    const parsedExpiry = expiry ? Number(expiry) : NaN;
    return {
      accessToken,
      refreshToken: refreshToken ?? undefined,
      accessTokenExpiresAt: Number.isFinite(parsedExpiry)
        ? parsedExpiry
        : undefined,
      userId: storedUserId ?? decodeSubject(accessToken),
    };
  }

  async function rawWrite(session: StoredAuthSession): Promise<void> {
    try {
      // Publish the access token last. Interrupted native writes leave an
      // orphan to clean up, rather than a login with mismatched account keys.
      await store.deleteItem(sessionStorageKeys.accessToken);
      if (
        typeof session.accessTokenExpiresAt === "number" &&
        Number.isFinite(session.accessTokenExpiresAt)
      ) {
        await store.setItem(
          sessionStorageKeys.expiresAt,
          String(Math.floor(session.accessTokenExpiresAt)),
        );
      } else await store.deleteItem(sessionStorageKeys.expiresAt);
      const userId = session.userId ?? decodeSubject(session.accessToken);
      if (userId) await store.setItem(sessionStorageKeys.userId, userId);
      else await store.deleteItem(sessionStorageKeys.userId);
      if (session.refreshToken)
        await store.setItem(
          sessionStorageKeys.refreshToken,
          session.refreshToken,
        );
      else await store.deleteItem(sessionStorageKeys.refreshToken);
      await store.setItem(sessionStorageKeys.accessToken, session.accessToken);
    } catch (error) {
      // Best effort cleanup also covers a failed replacement over an existing
      // account. Preserve the original storage error for the caller.
      try {
        await rawClear();
      } catch {
        /* A later load/clear can retry. */
      }
      throw error;
    }
  }

  function loadStoredAuthSession(): Promise<StoredAuthSession | null> {
    const epoch = getAuthEpoch();
    return serialize(async () => {
      if (epoch !== getAuthEpoch()) return null;
      const session = await rawLoad();
      return epoch === getAuthEpoch() ? session : null;
    });
  }

  function persistAuthSession(session: StoredAuthSession): Promise<void> {
    const epoch = getAuthEpoch();
    return serialize(async () => {
      if (epoch !== getAuthEpoch()) throw sessionChanged();
      await rawWrite(session);
      if (epoch !== getAuthEpoch()) throw sessionChanged();
    });
  }

  function clearStoredAuthSession(): Promise<void> {
    // Fence network work immediately, even if a native write holds the queue.
    advanceAuthEpoch();
    return serialize(rawClear);
  }

  function persistRefreshedAuthSession(
    session: StoredAuthSession,
    expectedEpoch: number,
    expectedRefreshToken: string,
  ): Promise<boolean> {
    return serialize(async () => {
      if (getAuthEpoch() !== expectedEpoch) return false;
      const currentRefreshToken = await store.getItem(
        sessionStorageKeys.refreshToken,
      );
      if (
        getAuthEpoch() !== expectedEpoch ||
        currentRefreshToken !== expectedRefreshToken
      )
        return false;
      await rawWrite(session);
      // Logout can fence a write already underway. Its queued clear removes
      // that write before any newly admitted login is persisted.
      return getAuthEpoch() === expectedEpoch;
    });
  }

  return {
    loadStoredAuthSession,
    persistAuthSession,
    clearStoredAuthSession,
    persistRefreshedAuthSession,
  };
}
