import type { ConsentSnapshot } from './analyticsConsentState';
import type { MobileEvent } from './telemetry-schema';
export type AnalyticsPermission = { account_id: string; enabled: boolean; revision: string };
export type RelayRequest = <T>(path: string, options?: { method?: 'GET' | 'POST' | 'PUT'; body?: unknown; expectedAccountId?: string; authorityGuard?: () => void }) => Promise<T>;
/** No event queue or positive permission cache. Every event obtains a new revision. */
export function createAnalyticsRelay(snapshot: () => ConsentSnapshot, request: RelayRequest) {
  let configured = false;
  let generation = 0;
  let admissions = 0;
  return {
    configure(enabled: boolean) { generation++; configured = enabled; },
    async capture(event: MobileEvent): Promise<void> {
      const start = snapshot();
      if (!configured || !start.isHydrated || !start.enabled || !start.accountId || admissions >= 16) return;
      const epoch = generation;
      const accountId = start.accountId;
      const guard = () => {
        const now = snapshot();
        if (!configured || epoch !== generation || now.revision !== start.revision || !now.enabled || now.accountId !== accountId)
          throw new Error('analytics_authority_changed');
      };
      admissions++;
      try {
        const options = { expectedAccountId: accountId, authorityGuard: guard };
        const permission = await request<AnalyticsPermission>('/users/me/preferences/analytics', options);
        guard();
        if (!permission.enabled || permission.account_id !== accountId || !permission.revision) return;
        await request('/users/me/analytics/events', { ...options, method: 'POST', body: { account_id: accountId, revision: permission.revision, event } });
      } catch { /* Optional analytics never affects product actions. */ }
      finally { admissions--; }
    },
  };
}

/** An account mutation always carries intent before the server writes. A late
 * response can never enable this device after local withdrawal or account switch. */
export async function saveAnalyticsPermission(accountId: string, enabled: boolean, request: RelayRequest,
  snapshot: () => ConsentSnapshot, setLocal: (enabled: boolean) => void): Promise<void> {
  if (snapshot().accountId !== accountId) throw new Error('analytics_account_changed');
  setLocal(false);
  const revision = snapshot().revision;
  const guard = () => {
    if (snapshot().accountId !== accountId || snapshot().revision !== revision)
      throw new Error('analytics_authority_changed');
  };
  const permission = await request<AnalyticsPermission>('/users/me/preferences/analytics', {
    method: 'PUT', expectedAccountId: accountId, authorityGuard: guard, body: { account_id: accountId, enabled },
  });
  guard();
  if (permission.account_id !== accountId || permission.enabled !== enabled) throw new Error('analytics_account_changed');
  setLocal(enabled);
}
