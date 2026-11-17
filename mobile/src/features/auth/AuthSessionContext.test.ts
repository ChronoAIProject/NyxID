import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { hookHarness, deferred } from '../assistant/assistantHookTestHarness';
function setup(initialPending?: Promise<unknown>) {
  let generation = 0, failures = 100, reads = 0, now = 0;
  let pending = initialPending;
  const timers = new Map<number, { due: number; run: () => void }>(); let next = 0;
  const listeners = new Set<(value: string) => void>();
  const appState = { currentState: 'active', addEventListener: (_: string, fn: (v: string) => void) => { listeners.add(fn); return {remove: () => listeners.delete(fn)}; } };
  const session = { accessToken: 'token', refreshToken: 'refresh', userId: 'account' };
  const h = hookHarness<any>({
    'react/jsx-runtime': {jsx: (type: unknown, props: unknown) => ({type, props})},
    'react-native': {AppState: appState},
    '@tanstack/react-query': {useQueryClient: () => client},
    '../../lib/auth/sessionStore': {authSessionGeneration: () => generation,
      loadStoredAuthSession: async () => { reads++; if (pending) await pending; if (failures-- > 0) throw new Error('locked'); return session; },
      persistAuthSession: async () => {generation++;}, clearStoredAuthSession: async () => {generation++;}},
    '../../lib/auth/authAccount': {getAuthAccountId: () => null, setAuthAccountId: () => {}},
    '../../lib/auth/authEpoch': {advanceAuthEpoch: () => {}},
    '../../lib/notifications/pushNotifications': {activatePushAfterLogin: async () => {}, deactivatePushOnLogout: async () => {}},
    '../../lib/notifications/approvalRefreshSignal': {clearPendingApprovalRefreshSignal: () => {}},
    '../../lib/api/http': {refreshAccessTokenIfNeeded: async () => {}, setSessionInvalidationListener: () => {}},
    '../../lib/env': {ALLOWED_EMAILS: [], isEmailAllowed: () => true},
    '../../lib/telemetry': Object.fromEntries(['reset','identify','shutdownTelemetry','initTelemetry','readExpoTelemetryConfig'].map(key => [key, () => {}])),
    '../../lib/consent': {useMobileConsent: () => ({enabled: false, isHydrated: true})},
  }, {__DEV__: false, setTimeout: (run: () => void, delay: number) => {const id=++next; timers.set(id,{run,due:now+delay}); return id;}, clearTimeout: (id: number) => timers.delete(id)});
  const client = {clear: () => {}, cancelQueries: async () => {}};
  const {AuthSessionProvider} = h.load(fileURLToPath(new URL('./AuthSessionContext.tsx', import.meta.url))) as any;
  h.mount(() => AuthSessionProvider({children: null}));
  return { h, value: () => h.value().props.value, reads: () => reads, recover: () => {failures=0;}, hold: (p: Promise<unknown>) => {pending=p;},
    foreground: () => {appState.currentState='active'; for(const fn of [...listeners]) fn('active');},
    background: () => {appState.currentState='background'; for(const fn of [...listeners]) fn('background');},
    advance: async (ms: number) => {now+=ms; for(const [id,timer] of [...timers]) if(timer.due<=now) {timers.delete(id); timer.run();} await h.settle();} };
}
test('P3 unreadable keychain recovers on a later foreground after repeated failures', async () => {
  const e=setup(); try {
    await e.h.settle(); for(let i=0;i<5;i++) await e.advance(1000);
    e.background(); e.recover(); e.foreground(); await e.h.settle();
    assert.equal(e.value().isAuthenticated,true); assert.equal(e.value().isRestoring,false);
  } finally {e.h.unmount();}
});
test('P3 active storage retries are bounded and keep recovering after the initial burst', async () => {
  const e=setup(); try {
    await e.h.settle(); for(let i=0;i<5;i++) await e.advance(1000);
    const before=e.reads(); await e.advance(1); assert.equal(e.reads(),before);
    e.recover(); await e.advance(30000);
    assert.equal(e.value().isAuthenticated,true);
  } finally {e.h.unmount();}
});
test('P3 pending restore never replaces an explicit login or sign-out', async () => {
  for(const action of ['signOut','signInWithSession']) {
    const gate=deferred<void>(); const e=setup(gate.promise); e.recover();
    try {
      await e.value()[action]({accessToken:'other',userId:'other'});
      gate.resolve(); await e.h.settle(); e.foreground(); await e.advance(30000);
      assert.equal(e.value().isAuthenticated,action==='signInWithSession');
      assert.equal(e.value().isRestoring,false);
    } finally {e.h.unmount();}
  }
});
