import type { StoredAuthSession } from './sessionCoordinator';
export type RefreshResult = { access_token: string; refresh_token: string; expires_in: number };
/** Single-flight is scoped to the captured login generation and token, while
 * committing rotated credentials uses the coordinator's serialized CAS. */
export function createSessionRefresh(deps: {
  load: () => Promise<StoredAuthSession | null>;
  commit: (expected: StoredAuthSession, next: StoredAuthSession) => Promise<boolean>;
  request: (token: string) => Promise<RefreshResult | null>;
  expiresAt: (seconds: number) => number | undefined;
}) {
  let flight: { generation: number; token: string; promise: Promise<RefreshResult | null> } | null = null;
  return async (): Promise<RefreshResult | null> => {
    const session = await deps.load();
    if (!session?.refreshToken || session.sessionGeneration === undefined) return null;
    if (flight?.generation === session.sessionGeneration && flight.token === session.accessToken) return flight.promise;
    const next = { generation: session.sessionGeneration, token: session.accessToken, promise: Promise.resolve<RefreshResult | null>(null) };
    next.promise = (async () => {
      const refreshed = await deps.request(session.refreshToken!);
      if (!refreshed) return null;
      const refreshToken = refreshed.refresh_token || session.refreshToken!;
      const committed = await deps.commit(session, { accessToken: refreshed.access_token, refreshToken,
        accessTokenExpiresAt: deps.expiresAt(refreshed.expires_in) });
      return committed ? { ...refreshed, refresh_token: refreshToken } : null;
    })().finally(() => { if (flight === next) flight = null; });
    flight = next;
    return next.promise;
  };
}
