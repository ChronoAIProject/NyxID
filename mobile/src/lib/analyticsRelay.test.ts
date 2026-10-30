import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createAnalyticsConsentState } from './analyticsConsentState';
import { createAnalyticsRelay, saveAnalyticsPermission, type RelayRequest } from './analyticsRelay';
import { createConsentPersistence } from './consentPersistence';
const event = { name: 'mobile.deep_link_opened', props: { link_type: 'challenge' } } as const;
const permission = { account_id: 'a', enabled: true, revision: 'r1' };
function setup() {
  const state = createAnalyticsConsentState(); state.hydrate(null); state.bindAccount('a'); state.setConsent(true);
  const calls: Array<{ path: string; options: any }> = [];
  let response = permission;
  const request: RelayRequest = async (path, options) => { options?.authorityGuard?.(); calls.push({ path, options }); return response as any; };
  const relay = createAnalyticsRelay(state.getSnapshot, request); relay.configure(true);
  return { state, relay, calls, setResponse: (value: typeof permission) => { response = value; } };
}
test('every mobile event obtains fresh permission, account/revision and bounded explicit metadata', async () => {
  const { relay, calls, setResponse } = setup();
  await relay.capture(event); await relay.capture(event);
  assert.equal(calls.length, 4);
  assert.deepEqual(calls[1]!.options.body, { account_id: 'a', revision: 'r1', event });
  assert.equal(calls[1]!.options.expectedAccountId, 'a');
  setResponse({ ...permission, enabled: false, revision: 'withdrawn' });
  await relay.capture(event); assert.equal(calls.length, 5);
});
test('mobile pending admissions cannot cross withdrawal/regrant, switch, or shutdown', async () => {
  for (const change of ['withdraw', 'switch', 'shutdown']) {
    const { state } = setup();
    let release!: (value: any) => void;
    let calls = 0;
    const request: RelayRequest = async () => { calls++; return new Promise((resolve) => { release = resolve; }); };
    const relay = createAnalyticsRelay(state.getSnapshot, request); relay.configure(true);
    const pending = relay.capture(event);
    if (change === 'withdraw') { state.setConsent(false); state.setConsent(true); }
    if (change === 'switch') { state.bindAccount('b'); state.setConsent(true); }
    if (change === 'shutdown') relay.configure(false);
    release(permission); await pending; assert.equal(calls, 1);
  }
});
test('missing permission, denied device, unknown account, failure, mismatched response and no config deny', async () => {
  const { relay, state, calls, setResponse } = setup();
  setResponse({ ...permission, account_id: 'b' }); await relay.capture(event); assert.equal(calls.length, 1);
  state.setConsent(false); await relay.capture(event); assert.equal(calls.length, 1);
  state.bindAccount(null); await relay.capture(event); assert.equal(calls.length, 1);
  const failed = createAnalyticsRelay(state.getSnapshot, async () => { throw new Error('down'); });
  failed.configure(true); state.bindAccount('a'); state.setConsent(true); await failed.capture(event);
  relay.configure(false); await relay.capture(event); assert.equal(calls.length, 1);
});
test('mobile admissions are bounded with no overflow backlog', async () => {
  const { state } = setup(); const releases: Array<(value: any) => void> = [];
  const relay = createAnalyticsRelay(state.getSnapshot, async () => new Promise((resolve) => { releases.push(resolve); }));
  relay.configure(true);
  const pending = Array.from({ length: 17 }, () => relay.capture(event));
  assert.equal(releases.length, 16);
  releases.forEach((release) => release({ ...permission, enabled: false }));
  await Promise.all(pending); assert.equal(releases.length, 16);
});
test('mutation binds account intent and late response cannot apply positive state', async () => {
  const { state } = setup(); let body: unknown;
  const request: RelayRequest = async (_path, options) => { body = options?.body; state.bindAccount('b'); return { ...permission, account_id: 'b' } as any; };
  await assert.rejects(saveAnalyticsPermission('a', true, request, state.getSnapshot, state.setConsent));
  assert.deepEqual(body, { account_id: 'a', enabled: true });
  assert.equal(state.getSnapshot().enabled, false);
});
test('Settings account toggle waits for acknowledgement and persists enable and withdrawal', async () => {
  const { state } = setup();
  const persist = createConsentPersistence(state, async () => {});
  let saved = Promise.resolve();
  const setLocal = (enabled: boolean) => { state.setConsent(enabled); saved = persist(); };
  for (const enabled of [true, false]) {
    let release!: (value: any) => void;
    let options: Parameters<RelayRequest>[1];
    const request: RelayRequest = async (_path, value) => {
      options = value;
      return new Promise((resolve) => { release = resolve; });
    };
    const pending = saveAnalyticsPermission('a', enabled, request, state.getSnapshot, setLocal);
    assert.equal(state.getSnapshot().enabled, false, 'local collection stops while saving');
    assert.equal(options?.method, 'PUT');
    assert.equal(options?.expectedAccountId, 'a');
    assert.deepEqual(options?.body, { account_id: 'a', enabled });
    // Settings persists the local denial asynchronously. A successful device
    // write may finish before the account API does; that is the same intent.
    await saved;
    options?.authorityGuard?.();
    release({ ...permission, enabled });
    await pending;
    await saved;
    assert.equal(state.getSnapshot().enabled, enabled);
    const restored = createAnalyticsConsentState();
    restored.bindAccount('a'); restored.hydrate(state.serialize());
    assert.equal(restored.getSnapshot().enabled, enabled);
  }
});
test('failed Settings withdrawal stops this device and reports the failure', async () => {
  const { state } = setup();
  await assert.rejects(saveAnalyticsPermission('a', false, async () => { throw new Error('offline'); },
    state.getSnapshot, state.setConsent), /offline/);
  assert.equal(state.getSnapshot().enabled, false);
});
test('a newer local choice fences a delayed Settings opt-in acknowledgement', async () => {
  const { state } = setup();
  let release!: (value: any) => void;
  const request: RelayRequest = async () => new Promise((resolve) => { release = resolve; });
  const pending = saveAnalyticsPermission('a', true, request, state.getSnapshot, state.setConsent);
  state.setConsent(false);
  release(permission);
  await assert.rejects(pending, /analytics_authority_changed/);
  assert.equal(state.getSnapshot().enabled, false);
});
test('mobile choices restore only for their account; late hydration preserves local withdrawal', () => {
  const { state } = setup(); const saved = state.serialize();
  const restored = createAnalyticsConsentState(); restored.bindAccount('b'); restored.hydrate(saved);
  assert.equal(restored.getSnapshot().enabled, false);
  restored.bindAccount('a'); assert.equal(restored.getSnapshot().enabled, true);
  const late = createAnalyticsConsentState(); late.bindAccount('a'); late.setConsent(false); late.hydrate(saved);
  assert.equal(late.getSnapshot().enabled, false);
  const legacy = createAnalyticsConsentState(); legacy.hydrate(JSON.stringify({ enabled: true, asked: true })); legacy.bindAccount('a');
  assert.equal(legacy.getSnapshot().enabled, false);
});
