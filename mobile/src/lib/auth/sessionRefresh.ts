import type { StoredAuthSession, RefreshCommitOutcome } from './sessionCoordinator';
import type { RefreshOutcome, RefreshWireOutcome } from './refreshOutcome';
export type { RefreshResult } from './refreshOutcome';
/** Single-flight belongs to the exact captured login and credential pair. */
export function createSessionRefresh(deps: {
  load: () => Promise<StoredAuthSession | null>;
  commit: (expected: StoredAuthSession, next: StoredAuthSession) => Promise<RefreshCommitOutcome>;
  request: (token: string) => Promise<RefreshWireOutcome>;
  expiresAt: (seconds: number) => number | undefined;
}) {
  let flight: { generation: number; access: string; refresh: string; promise: Promise<RefreshOutcome> } | null = null;
  return async (): Promise<RefreshOutcome> => {
    let session: StoredAuthSession | null;
    try { session = await deps.load(); }
    catch (cause) { return { kind: 'persistence_failed', cause }; }
    if (!session?.refreshToken || session.sessionGeneration === undefined) return { kind: 'no_credentials' };
    if (flight?.generation === session.sessionGeneration && flight.access === session.accessToken
      && flight.refresh === session.refreshToken) return flight.promise;
    const next = { generation: session.sessionGeneration, access: session.accessToken,
      refresh: session.refreshToken, promise: Promise.resolve<RefreshOutcome>({ kind: 'no_credentials' }) };
    next.promise = (async (): Promise<RefreshOutcome> => {
      let outcome: RefreshWireOutcome;
      try { outcome = await deps.request(session.refreshToken!); }
      catch (cause) { return { kind: 'transient', cause }; }
      if (outcome.kind !== 'refreshed') {
        try {
          const current = await deps.load();
          if (current?.sessionGeneration !== session.sessionGeneration
            || current?.accessToken !== session.accessToken || current?.refreshToken !== session.refreshToken)
            return { kind: 'superseded', current };
        } catch (cause) { return { kind: 'persistence_failed', cause }; }
        return outcome;
      }
      const pair = outcome.credentials;
      // Defense in depth for adapters other than the HTTP implementation.
      if (!pair.access_token?.trim() || !pair.refresh_token?.trim()
        || (pair.expires_in !== undefined && !Number.isFinite(pair.expires_in)))
        return { kind: 'transient', cause: 'incomplete_refresh_response' };
      let committed: RefreshCommitOutcome;
      try { committed = await deps.commit(session, { accessToken: pair.access_token,
        refreshToken: pair.refresh_token, accessTokenExpiresAt: deps.expiresAt(pair.expires_in ?? 0) }); }
      catch (cause) { return { kind: 'persistence_failed', cause }; }
      if (committed.kind === 'committed') return outcome;
      if (committed.kind === 'persistence_failed') return committed;
      try { return { kind: 'superseded', current: await deps.load() }; }
      catch (cause) { return { kind: 'persistence_failed', cause }; }
    })().finally(() => { if (flight === next) flight = null; });
    flight = next;
    return next.promise;
  };
}
