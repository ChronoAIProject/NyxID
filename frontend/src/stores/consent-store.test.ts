import { describe, it, expect, beforeEach, vi } from "vitest";

const STORAGE_KEY = "nyxid.telemetry_consent";

/**
 * These tests verify that the Settings → Privacy toggle actually
 * persists to localStorage — i.e. that flipping it is NOT cosmetic.
 * Without this, a toggle OFF would "work" inside a single page load
 * and then silently revert on reload, because `main.tsx` reads
 * `consentEnabled` from the same store at boot.
 *
 * happy-dom 20.x ships a stub `localStorage` without the Storage API
 * methods (it expects a `--localstorage-file` path to enable full
 * persistence). We back it with a plain in-memory map so the zustand
 * `persist` middleware has something real to read and write. This is
 * the same behavior as a browser's volatile localStorage within one
 * session, which is exactly what we need to verify.
 */

function installInMemoryLocalStorage() {
  const store = new Map<string, string>();
  const impl: Storage = {
    get length() {
      return store.size;
    },
    clear() {
      store.clear();
    },
    getItem(key: string) {
      return store.has(key) ? store.get(key)! : null;
    },
    key(index: number) {
      return Array.from(store.keys())[index] ?? null;
    },
    removeItem(key: string) {
      store.delete(key);
    },
    setItem(key: string, value: string) {
      store.set(key, String(value));
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    value: impl,
    configurable: true,
    writable: true,
  });
}

async function loadFreshStore() {
  // Reset the module graph so the `persist` middleware re-runs its
  // hydration from whatever is currently in localStorage. Simulates a
  // full page reload for the purpose of this test.
  const { useConsentStore } = await import("./consent-store");
  return useConsentStore;
}

beforeEach(() => {
  // Start each test from a known-clean slate: fresh in-memory storage
  // AND a reset module graph so the store's `persist` middleware
  // re-initializes from the empty storage.
  installInMemoryLocalStorage();
  vi.resetModules();
});

describe("account-bound device restrictions", () => {
  it("defaults off and refuses an unbound opt-in", async () => {
    const store = await loadFreshStore();
    store.getState().setConsent(true);
    expect(store.getState().enabled).toBe(false);
    expect(store.getState().asked).toBe(false);
  });

  it("does not transfer a choice between accounts or to a signed-out session", async () => {
    const store = await loadFreshStore();
    store.getState().bindAccount("account-a");
    store.getState().setConsent(true);
    store.getState().bindAccount("account-b");
    expect(store.getState()).toMatchObject({ enabled: false, asked: false });
    store.getState().setConsent(false);
    store.getState().bindAccount("account-a");
    expect(store.getState()).toMatchObject({ enabled: true, asked: true });
    store.getState().bindAccount(null);
    expect(store.getState()).toMatchObject({ enabled: false, asked: false });
    store.getState().bindAccount("account-b");
    expect(store.getState()).toMatchObject({ enabled: false, asked: true });
  });

  it("reload preserves choices but never a live identity or active opt-in", async () => {
    const store = await loadFreshStore();
    store.getState().bindAccount("account-a");
    store.getState().setConsent(true);
    expect(JSON.parse(localStorage.getItem("nyxid.telemetry_consent.v3.account-a")!)).toEqual({
      enabled: true, asked: true,
    });
    vi.resetModules();
    const reloaded = await loadFreshStore();
    expect(reloaded.getState()).toMatchObject({ accountId: null, enabled: false });
    reloaded.getState().bindAccount("account-b");
    expect(reloaded.getState().enabled).toBe(false);
    reloaded.getState().bindAccount("account-a");
    expect(reloaded.getState().enabled).toBe(true);
  });

  it("discards historical unbound positives instead of assigning them", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ version: 1, state: { enabled: true, asked: true } }));
    const store = await loadFreshStore();
    store.getState().bindAccount("account-a");
    expect(store.getState()).toMatchObject({ enabled: false, asked: false });
  });

  it("clears only this account's restriction", async () => {
    const store = await loadFreshStore();
    store.getState().bindAccount("account-a"); store.getState().setConsent(true);
    store.getState().bindAccount("account-b"); store.getState().setConsent(false);
    store.getState().clearConsent();
    expect(store.getState()).toMatchObject({ enabled: false, asked: false });
    store.getState().bindAccount("account-a");
    expect(store.getState().enabled).toBe(true);
  });
});

it("external sync is read-only and stale events cannot re-enable or overwrite a newer withdrawal", async () => {
  const store = await loadFreshStore();
  const { consentStorageKey, syncConsentStorage } = await import("./consent-store");
  store.getState().bindAccount("account-a"); store.getState().setConsent(true);
  const key = consentStorageKey("account-a");
  const positive = localStorage.getItem(key)!;
  const off = JSON.stringify({ enabled: false, asked: true });
  localStorage.setItem(key, off);
  const write = vi.spyOn(localStorage, "setItem");
  const remove = vi.spyOn(localStorage, "removeItem");
  const revision = store.getState().revision;
  window.dispatchEvent(new StorageEvent("storage", { key, newValue: positive }));
  expect(store.getState().enabled).toBe(false);
  expect(localStorage.getItem(key)).toBe(off);
  expect(store.getState().revision).toBeGreaterThan(revision);
  syncConsentStorage();
  expect(write).not.toHaveBeenCalled(); expect(remove).not.toHaveBeenCalled();
  store.getState().bindAccount("account-b"); store.getState().setConsent(true);
  expect(localStorage.getItem(key)).toBe(off);
  store.getState().bindAccount("account-a");
  expect(store.getState().enabled).toBe(false);
});

it("withdrawal then regrant before an event is processed still invalidates pending work", async () => {
  const store = await loadFreshStore();
  const { consentStorageKey } = await import("./consent-store");
  store.getState().bindAccount("account-a"); store.getState().setConsent(true);
  const revision = store.getState().revision;
  const key = consentStorageKey("account-a");
  const positive = localStorage.getItem(key)!;
  localStorage.setItem(key, JSON.stringify({ enabled: false, asked: true }));
  localStorage.setItem(key, positive);
  window.dispatchEvent(new StorageEvent("storage", { key, newValue: positive }));
  expect(store.getState().enabled).toBe(true);
  expect(store.getState().revision).toBeGreaterThan(revision);
  localStorage.setItem(key, "invalid");
  window.dispatchEvent(new StorageEvent("storage", { key, newValue: positive }));
  expect(store.getState().enabled).toBe(false);
});

it.each(["set", "remove"])("failed negative %s remains sticky across rebind and external sync until successful explicit opt-in", async (operation) => {
  const store = await loadFreshStore();
  const { syncConsentStorage } = await import("./consent-store");
  store.getState().bindAccount("account-a"); store.getState().setConsent(true);
  const method = operation === "set" ? "setItem" : "removeItem";
  const blocked = vi.spyOn(localStorage, method).mockImplementation(() => { throw new Error("storage blocked"); });
  if (operation === "set") store.getState().setConsent(false); else store.getState().clearConsent();
  expect(store.getState().enabled).toBe(false);
  store.getState().bindAccount("account-b"); store.getState().bindAccount("account-a");
  syncConsentStorage(); expect(store.getState().enabled).toBe(false);
  blocked.mockRestore();
  syncConsentStorage(); expect(store.getState().enabled).toBe(false);
  store.getState().setConsent(true); expect(store.getState().enabled).toBe(true);
});

it("reports persistence failure, keeps account denial isolated, and clears error on retry", async () => {
  const store = await loadFreshStore(); store.getState().bindAccount("a"); store.getState().setConsent(true);
  const original = localStorage.setItem;
  localStorage.setItem = () => { throw new Error("blocked"); };
  store.getState().setConsent(false);
  expect(store.getState()).toMatchObject({ enabled: false, persistenceError: true });
  store.getState().bindAccount("b"); expect(store.getState().persistenceError).toBe(false);
  store.getState().bindAccount("a"); expect(store.getState().persistenceError).toBe(true);
  localStorage.setItem = original; store.getState().setConsent(false);
  expect(store.getState()).toMatchObject({ enabled: false, persistenceError: false });
  expect(JSON.parse(localStorage.getItem("nyxid.telemetry_consent.v3.a")!)).toEqual({ enabled: false, asked: true });
});
