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

describe("consent-store persistence", () => {
  it("starts with enabled=false, asked=false when localStorage is empty", async () => {
    const useConsentStore = await loadFreshStore();
    const state = useConsentStore.getState();
    expect(state.enabled).toBe(false);
    expect(state.asked).toBe(false);
  });

  it("writes {enabled:true, asked:true} to localStorage when setConsent(true)", async () => {
    const useConsentStore = await loadFreshStore();
    useConsentStore.getState().setConsent(true);
    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!);
    // zustand `persist` wraps state in `{ state: {...}, version }`
    expect(parsed.state.enabled).toBe(true);
    expect(parsed.state.asked).toBe(true);
  });

  it("writes {enabled:false, asked:true} to localStorage when setConsent(false)", async () => {
    const useConsentStore = await loadFreshStore();
    useConsentStore.getState().setConsent(false);
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed = JSON.parse(raw!);
    expect(parsed.state.enabled).toBe(false);
    expect(parsed.state.asked).toBe(true);
  });

  it("rehydrates enabled=true from localStorage after a simulated reload", async () => {
    // Round 1: opt in.
    const store1 = await loadFreshStore();
    store1.getState().setConsent(true);

    // Round 2: simulate page reload — module graph reset, fresh import
    // should read back what Round 1 wrote to localStorage. The storage
    // itself is preserved because `installInMemoryLocalStorage` ran
    // only once in beforeEach, and module reset doesn't touch it.
    vi.resetModules();
    const store2 = await loadFreshStore();
    const state = store2.getState();
    expect(state.enabled).toBe(true);
    expect(state.asked).toBe(true);
  });

  it("rehydrates enabled=false from localStorage after a simulated reload (the critical withdrawal path)", async () => {
    // This is the test that matters: user toggles OFF in Settings,
    // closes the tab, reopens later. Their choice MUST survive.
    const store1 = await loadFreshStore();
    store1.getState().setConsent(true);
    store1.getState().setConsent(false);

    vi.resetModules();
    const store2 = await loadFreshStore();
    const state = store2.getState();
    expect(state.enabled).toBe(false);
    expect(state.asked).toBe(true);
  });
});
