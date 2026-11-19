/** Device restrictions belong to one account and only narrow server permission.
 * Each account has its own storage record: stale tabs cannot overwrite another
 * account's choice by saving an old aggregate snapshot. Legacy unbound opt-ins
 * are deliberately not imported. Binding and external synchronization never write.
 */
import { create } from 'zustand';

type Choice = { enabled: boolean; asked: boolean; policy?: 1|2 };
const OFF: Choice = { enabled: false, asked: false, policy: undefined };
// A failed withdrawal must not be undone by rereading an older disk opt-in.
// Only a successful explicit positive write clears this in-memory denial.
const failedDenials = new Map<string, Choice>();
const persistenceFailures = new Set<string>();
export const consentStorageKey = (accountId: string) => `nyxid.telemetry_consent.v3.${encodeURIComponent(accountId)}`;
function readChoice(accountId: string | null): Choice {
  if (!accountId) return OFF;
  const failedDenial = failedDenials.get(accountId);
  if (failedDenial) return failedDenial;
  try {
    const value: unknown = JSON.parse(localStorage.getItem(consentStorageKey(accountId)) ?? 'null');
    if (value && typeof value === 'object' && 'enabled' in value && 'asked' in value
        && typeof value.enabled === 'boolean' && typeof value.asked === 'boolean')
      return { enabled: value.enabled, asked: value.asked, policy: 'policy' in value && value.policy === 2 ? 2 : 1 };
  } catch { /* Missing, blocked or corrupt storage fails closed. */ }
  return OFF;
}
export interface ConsentState extends Choice {
  revision: number;
  persistenceError: boolean;
  accountId: string | null;
  bindAccount(accountId: string | null): void;
  setConsent(enabled: boolean, policy?: 1|2, options?: { markAsked?: boolean }): void;
  clearConsent(): void;
}
export const useConsentStore = create<ConsentState>((set, get) => ({
  accountId: null, ...OFF, revision: 0, persistenceError: false,
  bindAccount(accountId) {
    const state = get();
    const choice = readChoice(accountId);
    const persistenceError = !!accountId && persistenceFailures.has(accountId);
    // Routine profile refreshes preserve in-flight consent when the account
    // and its device choice are unchanged. Real choices and storage events
    // still invalidate pending work, even on withdraw/regrant cycles.
    if (accountId === state.accountId && choice.enabled === state.enabled
        && choice.asked === state.asked && choice.policy === state.policy && persistenceError === state.persistenceError) return;
    set({ accountId, persistenceError, ...choice, revision: state.revision + 1 });
  },
  setConsent(enabled, policy = 1, options) {
    const { accountId, asked } = get();
    if (!accountId) return;
    // A temporary denial while an account choice is saving is not an answer.
    const choice = { enabled, asked: options?.markAsked === false ? asked : true, policy };
    try { localStorage.setItem(consentStorageKey(accountId), JSON.stringify(choice)); }
    catch { /* An unsaved opt-in must not appear durable. Withdrawal still wins. */
      const denied = { ...choice, enabled: false };
      failedDenials.set(accountId, denied);
      persistenceFailures.add(accountId);
      set({ ...denied, persistenceError: true, revision: get().revision + 1 });
      return;
    }
    persistenceFailures.delete(accountId);
    if (enabled) failedDenials.delete(accountId);
    else if (failedDenials.has(accountId)) failedDenials.set(accountId, choice);
    set({ ...choice, persistenceError: false, revision: get().revision + 1 });
  },
  clearConsent() {
    const { accountId } = get();
    if (accountId) { try { localStorage.removeItem(consentStorageKey(accountId)); } catch { failedDenials.set(accountId, OFF); persistenceFailures.add(accountId); } }
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
