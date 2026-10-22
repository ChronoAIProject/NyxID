import type { StoredAuthSession, SessionInvalidationToken } from './sessionCoordinator';
/** The registered listener owns the destructive compare-and-clear. Remote
 * cleanup always receives the invalidated pair and runs outside the queue. */
export function createSessionInvalidationRegistry(fallback: (expected: StoredAuthSession) => Promise<unknown>) {
  let listener: ((expected: StoredAuthSession) => Promise<void>) | null = null;
  return {
    setListener(next: typeof listener) { listener = next; },
    async invalidate(expected: StoredAuthSession) {
      if (listener) await listener(expected); else await fallback(expected);
    },
  };
}
export function createSessionInvalidationHandler(deps: {
  compareAndClear: (expected: StoredAuthSession) => Promise<SessionInvalidationToken | null>;
  remoteCleanup: (previous: StoredAuthSession) => Promise<boolean>;
  cleanupIfInvalidated: (token: SessionInvalidationToken, cleanup: () => Promise<void> | void) => Promise<boolean>;
  localCleanup: (remoteSucceeded: boolean) => Promise<void>;
  signedOut: () => void;
}) {
  return async (expected: StoredAuthSession): Promise<void> => {
    const token = await deps.compareAndClear(expected);
    if (!token) return;
    let remoteSucceeded = false;
    try { remoteSucceeded = await deps.remoteCleanup(token.previous); } catch { /* best effort after the durable clear */ }
    await deps.cleanupIfInvalidated(token, async () => {
      await deps.localCleanup(remoteSucceeded);
    });
    // The second serialized check prevents an awaited local cleanup from
    // transitioning a newer login's UI to signed-out.
    await deps.cleanupIfInvalidated(token, deps.signedOut);
  };
}
