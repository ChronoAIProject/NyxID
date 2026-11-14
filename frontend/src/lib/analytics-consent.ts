import { apiClient } from './api-client';
import { disableTelemetry } from './telemetry';
import { useAuthStore } from '@/stores/auth-store';
import { useConsentStore } from '@/stores/consent-store';

export interface AnalyticsConsent {
  current_policy_version?: string;
  diagnostics_offered?: boolean;
  diagnostics_available?: boolean;
  policy_version?: string | null;
  enabled: boolean;
  changed_at: string | null;
  revision: string;
  account_id: string;
}

export function accountGuard(accountId: string): () => void {
  return () => {
    if (useAuthStore.getState().user?.id !== accountId) {
      throw new Error('Account changed; try again from the current account.');
    }
  };
}

/** Display state only. Event admission must obtain a fresh revision per event. */
export async function readAnalyticsConsent(accountId: string): Promise<AnalyticsConsent> {
  const result = await apiClient<AnalyticsConsent>('/users/me/preferences/analytics', { authorityGuard: accountGuard(accountId) });
  if (result.account_id !== accountId) throw new Error('Account preference response did not match.');
  return result;
}

/** A deliberate choice changes account permission and this device's restriction. */
export async function saveAnalyticsConsent(accountId: string, enabled: boolean, acknowledged: 1|2 = 1): Promise<AnalyticsConsent> {
  const guard = accountGuard(accountId);
  guard();
  // Failed withdrawal must still stop this device immediately. Opt-in waits for
  // durable acknowledgement and cannot inherit a previous account's preference.
  useConsentStore.getState().setConsent(false);
  disableTelemetry();
  const revision = useConsentStore.getState().revision;
  const intentGuard = () => {
    guard();
    if (useConsentStore.getState().revision !== revision) {
      throw new Error('Analytics choice changed; try again from the current settings.');
    }
  };
  const policy = acknowledged === 2 && (await readAnalyticsConsent(accountId)).current_policy_version === "2" ? 2 : 1;
  intentGuard();
  const result = await apiClient<AnalyticsConsent>('/users/me/preferences/analytics', {
    method: 'PUT', body: { account_id: accountId, enabled, ...(policy === 2 ? { policy_version: "2" } : {}) }, authorityGuard: intentGuard,
  });
  intentGuard();
  if (result.account_id !== accountId || result.enabled !== enabled) throw new Error('Account preference response did not match.');
  useConsentStore.getState().setConsent(enabled && result.enabled, policy);
  return result;
}
