/** Device restrictions belong to one account and only narrow server permission.
 * Each account has its own storage record: stale tabs cannot overwrite another
 * account's choice by saving an old aggregate snapshot. Legacy unbound opt-ins
 * are deliberately not imported. Binding and external synchronization never write.
 */
import { create } from 'zustand';

type Choice = { enabled: boolean; asked: boolean };
const OFF: Choice = { enabled: false, asked: false };
// A failed withdrawal must not be undone by rereading an older disk opt-in.
// Only a successful explicit positive write clears this in-memory denial.
const failedDenials = new Set<string>();
const persistenceFailures = new Set<string>();
export const consentStorageKey = (accountId: string) => `nyxid.telemetry_consent.v3.${encodeURIComponent(accountId)}`;
function readChoice(accountId: string | null): Choice {
  if (!accountId) return OFF;
  if (failedDenials.has(accountId)) return { enabled: false, asked: true };
  try {
    const value: unknown = JSON.parse(localStorage.getItem(consentStorageKey(accountId)) ?? 'null');
    if (value && typeof value === 'object' && 'enabled' in value && 'asked' in value
        && typeof value.enabled === 'boolean' && typeof value.asked === 'boolean')
      return { enabled: value.enabled, asked: value.asked };
  } catch { /* Missing, blocked or corrupt storage fails closed. */ }
  return OFF;
}
export interface ConsentState extends Choice {
  revision: number;
  persistenceError: boolean;
  accountId: string | null;
  bindAccount(accountId: string | null): void;
  setConsent(enabled: boolean): void;
  clearConsent(): void;
}
export const useConsentStore = create<ConsentState>((set, get) => ({
  accountId: null, ...OFF, revision: 0, persistenceError: false,
  bindAccount(accountId) {
    set({ accountId, persistenceError: !!accountId && persistenceFailures.has(accountId), ...readChoice(accountId), revision: get().revision + 1 });
  },
  setConsent(enabled) {
    const { accountId } = get();
    if (!accountId) return;
    const choice = { enabled, asked: true };
    try { localStorage.setItem(consentStorageKey(accountId), JSON.stringify(choice)); }
    catch { /* An unsaved opt-in must not appear durable. Withdrawal still wins. */
      failedDenials.add(accountId);
      persistenceFailures.add(accountId);
      set({ enabled: false, asked: true, persistenceError: true, revision: get().revision + 1 });
      return;
    }
    persistenceFailures.delete(accountId);
    if (enabled) failedDenials.delete(accountId);
    set({ ...choice, persistenceError: false, revision: get().revision + 1 });
  },
  clearConsent() {
    const { accountId } = get();
    if (accountId) { try { localStorage.removeItem(consentStorageKey(accountId)); } catch { failedDenials.add(accountId); persistenceFailures.add(accountId); } }
    set({ ...OFF, persistenceError: !!accountId && persistenceFailures.has(accountId), revision: get().revision + 1 });
  },
}));

/** Read the latest record, not StorageEvent.newValue: events may arrive after a
 * newer withdrawal. Even a withdrawal/regrant already processed by another tab
 * invalidates pending admissions. Zustand here has no persistence middleware.
 */
export function syncConsentStorage(): void {
  const state = useConsentStore.getState();
  useConsentStore.setState({ ...readChoice(state.accountId), revision: state.revision + 1 });
}
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (event) => {
    const accountId = useConsentStore.getState().accountId;
    if (event.key === null || (accountId && event.key === consentStorageKey(accountId))) syncConsentStorage();
  });
}
