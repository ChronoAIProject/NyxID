import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createAnalyticsConsentState } from './analyticsConsentState';
import { createConsentPersistence } from './consentPersistence';

test('successful storage acknowledgement preserves authority; failed storage invalidates it', async () => {
  const state = createAnalyticsConsentState(); state.hydrate(null); state.bindAccount('a'); state.setConsent(true);
  const before = state.getSnapshot().revision;
  await createConsentPersistence(state, async () => {})();
  assert.equal(state.getSnapshot().revision, before);
  assert.equal(state.getSnapshot().enabled, true);
  await createConsentPersistence(state, async () => { throw new Error('storage unavailable'); })();
  assert.ok(state.getSnapshot().revision > before);
  assert.equal(state.getSnapshot().enabled, false);
});

test('failed A write after switch never changes B, retains A denial and offers retry', async () => {
  const state = createAnalyticsConsentState(); state.hydrate(null); state.bindAccount('a'); state.setConsent(true);
  let reject!: (error: Error) => void;
  let started!: () => void;
  const waiting = new Promise<void>((resolve) => { started = resolve; });
  let fail = true; let disk = state.serialize();
  const persist = createConsentPersistence(state, async (serialized) => {
    if (fail) { started(); await new Promise<void>((_resolve, failure) => { reject = failure; }); }
    disk = serialized;
  });
  const pending = persist(); await waiting;
  state.bindAccount('b'); state.setConsent(true); const before = state.getSnapshot();
  reject(new Error('storage unavailable')); await pending;
  assert.deepEqual(state.getSnapshot(), before);
  state.bindAccount('a'); assert.equal(state.getSnapshot().enabled, false); assert.equal(state.getSnapshot().persistenceError, true);
  state.hydrate(disk); state.bindAccount('b'); state.bindAccount('a');
  assert.equal(state.getSnapshot().enabled, false);
  fail = false; await persist();
  assert.equal(state.getSnapshot().persistenceError, false);
  const restored = createAnalyticsConsentState(); restored.hydrate(disk); restored.bindAccount('a');
  assert.equal(restored.getSnapshot().enabled, false);
  restored.bindAccount('b'); assert.equal(restored.getSnapshot().enabled, true);
});

test('stale failed revision cannot undo newer explicit choice and serialized successor includes denial', async () => {
  const state = createAnalyticsConsentState(); state.hydrate(null); state.bindAccount('a'); state.setConsent(true);
  let release!: () => void; let started!: () => void;
  const waiting = new Promise<void>((resolve) => { started = resolve; }); let first = true; let disk = '';
  const persist = createConsentPersistence(state, async (serialized) => {
    if (first) { first = false; started(); await new Promise<void>((resolve) => { release = resolve; }); throw new Error('quota'); }
    disk = serialized;
  });
  const pending = persist(); await waiting;
  state.setConsent(false); const newer = persist(); release(); await pending; await newer;
  assert.equal(state.getSnapshot().enabled, false); assert.equal(state.getSnapshot().persistenceError, false);
  const restored = createAnalyticsConsentState(); restored.hydrate(disk); restored.bindAccount('a');
  assert.equal(restored.getSnapshot().enabled, false);
});
