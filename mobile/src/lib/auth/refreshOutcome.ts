import type { StoredAuthSession } from './sessionCoordinator';
export type RefreshResult = { access_token: string; refresh_token: string; expires_in?: number };
export type RefreshWireOutcome =
  | { kind: 'refreshed'; credentials: RefreshResult }
  | { kind: 'rejected'; status: number }
  | { kind: 'transient'; cause?: unknown };
export type RefreshOutcome = RefreshWireOutcome
  | { kind: 'superseded'; current: StoredAuthSession | null }
  | { kind: 'no_credentials' }
  | { kind: 'persistence_failed'; cause?: unknown };
export class RefreshRetryableError extends Error {
  readonly retryable = true;
  constructor(public readonly outcome: RefreshOutcome) {
    super('Session renewal is temporarily unavailable; retry later.');
    this.name = 'RefreshRetryableError';
  }
}
