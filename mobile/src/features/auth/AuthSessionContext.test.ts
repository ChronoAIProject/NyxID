import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createSessionCoordinator, SESSION_RECORD_KEY } from '../../lib/auth/sessionCoordinator';
import { hookHarness, deferred } from '../assistant/assistantHookTestHarness';
import { translate } from '../../i18n/translate';
import type { Language } from '../../i18n/language';
function setup(initialPending?: Promise<unknown>, config?: {language: Language; profile: 'denied' | 'unavailable'}) {
  let generation = 0, failures = 100, reads = 0, now = 0;
  let pending = initialPending;
  const timers = new Map<number, { due: number; run: () => void }>(); let next = 0;
  const listeners = new Set<(value: string) => void>();
  const appState = { currentState: 'active', addEventListener: (_: string, fn: (v: string) => void) => { listeners.add(fn); return {remove: () => listeners.delete(fn)}; } };
  const session = { accessToken: 'token', refreshToken: 'refresh', userId: 'account' };
  let stored: typeof session | null = session;
  const h = hookHarness<any>({
    "../../lib/crashReporting/runtime": {crashLifecycle: null},
    ...(config ? {'../../i18n/LanguageProvider': {useAppLanguage: () => ({language:config.language,t:(phrase:string)=>translate(config.language,phrase)})}} : {}),
    'react/jsx-runtime': {jsx: (type: unknown, props: unknown) => ({type, props})},
    'react-native': {AppState: appState},
    '@tanstack/react-query': {useQueryClient: () => client},
    '../../lib/auth/sessionStore': {authSessionGeneration: () => generation,peekStoredAuthSession: () => stored,
      loadStoredAuthSession: async () => { reads++; if (pending) await pending; if (failures-- > 0) throw new Error('locked'); return stored; },
      persistAuthSession: async () => {generation++;stored=session;}, clearStoredAuthSession: async () => {generation++;stored=null;}},
    '../../lib/auth/authAccount': {getAuthAccountId: () => null, setAuthAccountId: () => {}},
    '../../lib/auth/authEpoch': {advanceAuthEpoch: () => {}},
    '../../lib/notifications/pushNotifications': {capturePushLogout: async () => ({}), activatePushAfterLogin: async () => {}, deactivatePushOnLogout: async () => {}},
    '../../lib/notifications/approvalRefreshSignal': {clearPendingApprovalRefreshSignal: () => {}},
    '../../lib/api/http': {getCurrentUserProfileRequest: async () => {if(config?.profile==='unavailable')throw Error('offline');return {email:'denied@example.test'};}, refreshAccessTokenIfNeeded: async () => {}, setSessionInvalidationListener: () => {}},
    '../../lib/env': {ALLOWED_EMAILS: config ? ['allowed@example.test'] : [], isEmailAllowed: () => !config},
    '../../lib/telemetry': Object.fromEntries(['reset','identify','shutdownTelemetry','initTelemetry','readExpoTelemetryConfig'].map(key => [key, () => {}])),
    '../../lib/consent': {useMobileConsent: () => ({enabled: false, isHydrated: true})},
  }, {__DEV__: false, setTimeout: (run: () => void, delay: number) => {const id=++next; timers.set(id,{run,due:now+delay}); return id;}, clearTimeout: (id: number) => timers.delete(id)});
  const client = {clear: () => {}, cancelQueries: async () => {}};
  const {AuthSessionProvider} = h.load(fileURLToPath(new URL('./AuthSessionContext.tsx', import.meta.url))) as any;
  h.mount(() => AuthSessionProvider({children: null}));
  return { h, stored: () => stored, value: () => h.value().props.value, reads: () => reads, recover: () => {failures=0;}, hold: (p: Promise<unknown>) => {pending=p;},
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


test('M2622-03 durable sign-out retries a failed tombstone without waiting for push teardown', async () => {
  const values = new Map<string, string>(); let fail = false, account: string | null = 'account';
  const storage = { getItemAsync: async (k: string) => values.get(k) ?? null,
    setItemAsync: async (k: string, v: string) => { if (fail) throw Error('disk'); values.set(k,v); },
    deleteItemAsync: async (k: string) => { values.delete(k); } };
  const coordinator = createSessionCoordinator(storage, () => {});
  await coordinator.persist({accessToken:'token',refreshToken:'refresh',userId:'account'});
  const push = deferred<void>(); let teardowns = 0;
  const crashDisarms: Array<{generation:number; account:string|null}> = [];
  const client = {clear() {}, cancelQueries: async () => {}};
  const h = hookHarness<any>({
    "../../lib/crashReporting/runtime": {crashLifecycle: {setRestoring: () => {}, disarm: () => {crashDisarms.push({generation:coordinator.generation(),account});}}},
    'react/jsx-runtime': {jsx: (type: unknown, props: unknown) => ({type,props})},
    'react-native': {AppState: {currentState:'active',addEventListener: () => ({remove() {}})}},
    '@tanstack/react-query': {useQueryClient: () => client},
    '../../lib/auth/sessionStore': {authSessionGeneration: coordinator.generation,peekStoredAuthSession: coordinator.peek,loadStoredAuthSession: coordinator.load,persistAuthSession: coordinator.persist,clearStoredAuthSession: coordinator.clear},
    '../../lib/auth/authAccount': {getAuthAccountId: () => account,setAuthAccountId: (v: string | null) => {account=v;}},
    '../../lib/auth/authEpoch': {advanceAuthEpoch() {}},
    '../../lib/notifications/pushNotifications': {activatePushAfterLogin: async () => {},capturePushLogout: async () => ({accessToken:'token'}),deactivatePushOnLogout: async () => {teardowns++; await push.promise;}},
    '../../lib/notifications/approvalRefreshSignal': {clearPendingApprovalRefreshSignal() {}},
    '../../lib/api/http': {refreshAccessTokenIfNeeded: async () => {},setSessionInvalidationListener() {}},
    '../../lib/env': {ALLOWED_EMAILS:[],isEmailAllowed: () => true},
    '../../lib/telemetry': Object.fromEntries(['reset','identify','shutdownTelemetry','initTelemetry','readExpoTelemetryConfig'].map(k => [k, () => {}])),
    '../../lib/consent': {useMobileConsent: () => ({enabled:false,isHydrated:true})},
  }, {__DEV__:false});
  const {AuthSessionProvider} = h.load(fileURLToPath(new URL('./AuthSessionContext.tsx',import.meta.url))) as any;
  h.mount(() => AuthSessionProvider({children:null}));
  try {
    await h.settle(); const value = () => h.value().props.value;
    fail=true;
    // A held push must not hide the durable-write rejection.
    let rejected=false; const first=value().signOut().catch(() => {rejected=true;});
    await h.settle(); assert.equal(rejected,true); await first;
    assert.equal(crashDisarms.length,0,'failed durable clear retains crash authority for admitted account');
    assert.equal(teardowns,0); assert.equal(value().isAuthenticated,true); assert.equal(account,'account');
    assert.ok(await createSessionCoordinator(storage,()=>{}).load());
    fail=false; let complete=false; const retry=value().signOut().then(() => {complete=true;});
    await h.settle(); assert.equal(complete,true); await retry;
    assert.equal(crashDisarms.length,1,'successful clear disarms diagnostics once');
    assert.equal(crashDisarms[0]!.account,'account','disarm precedes publishing signed-out owner');
    assert.equal(value().isAuthenticated,false); assert.equal(account,null); assert.equal(teardowns,1);
    assert.equal(await createSessionCoordinator(storage,()=>{}).load(),null);
    assert.ok(!values.has(SESSION_RECORD_KEY) || JSON.parse(values.get(SESSION_RECORD_KEY)!).session===null);
  } finally {push.resolve(); h.unmount();}
});


test('M2622-07 allowlist denial and profile outage remain distinct in every app language', async () => {
  for(const language of ['en','zh-CN','zh-TW'] as const) {
    for(const profile of ['denied','unavailable'] as const) {
      const e=setup(undefined,{language,profile});
      try {
        await assert.rejects(e.value().signInWithSession({accessToken:'new',userId:'new'}),(error:any)=>{
          assert.equal(error.message,translate(language,profile==='denied'?'Access restricted. Your account is not authorized for this app.':'Unable to verify account access. Please try again.'));
          return true;
        });
        assert.equal(e.stored(),null,'both denied and unverifiable credentials must be cleared');
        await e.h.settle();assert.equal(e.value().isAuthenticated,false);
      } finally {e.h.unmount();}
    }
  }
});

test('M2622-03 push capture faults cannot gate sign-out and teardown captures the rotated access token', {timeout: 5000}, async (t) => {
  for (const mode of ['rotation', 'reject', 'hang'] as const) await t.test(mode, async () => {
    const jwt = (label: string) => `e30.${Buffer.from(JSON.stringify({sub: 'a', label})).toString('base64url')}.sig`;
    const values = new Map<string, string>();
    const storage = {getItemAsync: async (key: string) => values.get(key) ?? null,
      setItemAsync: async (key: string, value: string) => {values.set(key, value);},
      deleteItemAsync: async (key: string) => {values.delete(key);}};
    const coordinator = createSessionCoordinator(storage, () => {});
    await coordinator.persist({accessToken: jwt('old'), refreshToken: 'old'});
    let account: string | null = 'a';
    let capturedAccess: string | undefined;
    const teardowns: any[] = [];
    const client = {clear() {}, cancelQueries: async () => {}};
    const h = hookHarness<any>({
    "../../lib/crashReporting/runtime": {crashLifecycle: null},
      'react/jsx-runtime': {jsx: (type: unknown, props: unknown) => ({type, props})},
      'react-native': {AppState: {currentState: 'active', addEventListener: () => ({remove() {}})}},
      '@tanstack/react-query': {useQueryClient: () => client},
      '../../lib/auth/sessionStore': {authSessionGeneration: coordinator.generation, loadStoredAuthSession: coordinator.load,
        peekStoredAuthSession: coordinator.peek, persistAuthSession: coordinator.persist, clearStoredAuthSession: coordinator.clear},
      '../../lib/auth/authAccount': {getAuthAccountId: () => account, setAuthAccountId: (value: string | null) => {account = value;}},
      '../../lib/auth/authEpoch': {advanceAuthEpoch() {}},
      '../../lib/notifications/pushNotifications': {activatePushAfterLogin: async () => {},
        capturePushLogout: async (accessToken?: string) => {capturedAccess = accessToken;
          if (mode === 'reject') throw Error('push storage unavailable');
          if (mode === 'hang') return new Promise(() => {});
          return {accessToken, registration: null};},
        deactivatePushOnLogout: async (capture: unknown) => {teardowns.push(capture);return true;}},
      '../../lib/notifications/approvalRefreshSignal': {clearPendingApprovalRefreshSignal() {}},
      '../../lib/api/http': {refreshAccessTokenIfNeeded: async () => {}, setSessionInvalidationListener() {}},
      '../../lib/env': {ALLOWED_EMAILS: [], isEmailAllowed: () => true},
      '../../lib/telemetry': Object.fromEntries(['reset', 'identify', 'shutdownTelemetry', 'initTelemetry', 'readExpoTelemetryConfig'].map(key => [key, () => {}])),
      '../../lib/consent': {useMobileConsent: () => ({enabled: false, isHydrated: true})},
    }, {__DEV__: false});
    const {AuthSessionProvider} = h.load(fileURLToPath(new URL('./AuthSessionContext.tsx', import.meta.url))) as any;
    h.mount(() => AuthSessionProvider({children: null}));
    try {
      await h.settle();
      assert.equal(h.value().props.value.isAuthenticated, true);
      await coordinator.commitRefresh((await coordinator.load())!, {accessToken: jwt('rotated'), refreshToken: 'successor'});
      await h.value().props.value.signOut();
      await h.settle();
      assert.equal(capturedAccess, jwt('rotated'));
      assert.equal(teardowns[0]?.accessToken, jwt('rotated'));
      assert.equal(h.value().props.value.isAuthenticated, false);
      assert.equal(await createSessionCoordinator(storage, () => {}).load(), null);
    } finally {h.unmount();}
  });
});
