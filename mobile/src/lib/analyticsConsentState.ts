/** Local restrictions only; server permission is never cached here. */
export type DeviceChoice = { enabled: boolean; asked: boolean };
export type ConsentSnapshot = DeviceChoice & { accountId: string | null; revision: number; isHydrated: boolean; persistenceError: boolean };
const OFF: DeviceChoice = { enabled: false, asked: false };
export function createAnalyticsConsentState() {
  let choices: Record<string, DeviceChoice> = {};
  const versions: Record<string, number> = {};
  const failures = new Set<string>();
  let state: ConsentSnapshot = { ...OFF, accountId: null, revision: 0, isHydrated: false, persistenceError: false };
  const listeners = new Set<() => void>();
  const update = (patch: Partial<ConsentSnapshot>, invalidate = true) => {
    state = { ...state, ...patch, revision: state.revision + (invalidate ? 1 : 0) };
    listeners.forEach((listener) => listener());
  };
  return {
    getSnapshot: () => state,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    bindAccount(accountId: string | null) {
      if (state.accountId === accountId) return;
      update({ accountId, persistenceError: !!accountId && failures.has(accountId), ...(accountId ? choices[accountId] ?? OFF : OFF) });
    },
    hydrate(raw: string | null) {
      try {
        const parsed = JSON.parse(raw ?? 'null');
        // Never assign a legacy unbound positive to a newly logged-in user.
        if (parsed?.version === 2 && parsed.choices && typeof parsed.choices === 'object') {
          choices = { ...Object.fromEntries(Object.entries(parsed.choices).filter(([, value]) => {
            const v = value as Partial<DeviceChoice> | null;
            return v && typeof v.enabled === 'boolean' && typeof v.asked === 'boolean';
          })) as Record<string, DeviceChoice>, ...choices };
        }
      } catch { /* Corrupt storage defaults off. */ }
      update({ isHydrated: true, ...(state.accountId ? choices[state.accountId] ?? OFF : OFF) });
    },
    setConsent(enabled: boolean) {
      if (!state.accountId) return;
      versions[state.accountId] = (versions[state.accountId] ?? 0) + 1;
      const choice = { enabled, asked: true };
      choices = { ...choices, [state.accountId]: choice };
      update(choice);
    },
    clearConsent() {
      if (state.accountId) { versions[state.accountId] = (versions[state.accountId] ?? 0) + 1; choices = { ...choices, [state.accountId]: OFF }; }
      update(OFF);
    },
    persistenceIntent: () => state.accountId ? { accountId: state.accountId, revision: versions[state.accountId] ?? 0 } : null,
    finishPersistence(intent: { accountId: string; revision: number }, success: boolean) {
      if ((versions[intent.accountId] ?? 0) !== intent.revision) return;
      if (success) failures.delete(intent.accountId);
      else {
        failures.add(intent.accountId);
        // Fail closed for the captured account only, including when it is no
        // longer selected. Retain this denial across rebind and late hydration.
        choices = { ...choices, [intent.accountId]: { enabled: false, asked: true } };
      }
      // A successful write acknowledges the same choice. It must not revoke
      // a pending account toggle's authority merely because storage finished.
      // Failure changes authority to denial and still invalidates pending work.
      if (state.accountId === intent.accountId) update({ ...choices[intent.accountId], persistenceError: !success }, !success);
    },
    serialize: () => JSON.stringify({ version: 2, choices }),
  };
}
export const analyticsConsentState = createAnalyticsConsentState();
export function analyticsDeviceHeaders(): Record<string, string> {
  const state = analyticsConsentState.getSnapshot();
  return state.isHydrated && state.enabled && state.accountId
    ? { 'X-NyxID-Analytics': 'enabled', 'X-NyxID-Analytics-Account': state.accountId } : {};
}
