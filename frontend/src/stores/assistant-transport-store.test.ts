import { beforeEach, describe, expect, it, vi } from "vitest";

const STORAGE_KEY = "nyxid-assistant-transport-mode";

/**
 * The toggle is only useful if the choice survives a reload, so these tests
 * exercise the same persist round-trip as `consent-store.test.ts` — including
 * its in-memory localStorage backing, because happy-dom 20.x ships a stub
 * `localStorage` without the Storage API methods.
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
  const { useAssistantTransportStore } = await import(
    "./assistant-transport-store"
  );
  return useAssistantTransportStore;
}

beforeEach(() => {
  installInMemoryLocalStorage();
  vi.resetModules();
});

describe("assistant-transport-store", () => {
  it("defaults to the chat transport when localStorage is empty", async () => {
    const store = await loadFreshStore();
    expect(store.getState().mode).toBe("chat");
  });

  it("persists a switch to completions", async () => {
    const store = await loadFreshStore();
    store.getState().setMode("completions");
    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!) as { state: { mode: string } };
    expect(parsed.state.mode).toBe("completions");
  });

  it("rehydrates the selected mode after a simulated reload", async () => {
    const first = await loadFreshStore();
    first.getState().setMode("completions");

    vi.resetModules();
    const second = await loadFreshStore();
    expect(second.getState().mode).toBe("completions");

    // And back again — the round trip works in both directions.
    second.getState().setMode("chat");
    vi.resetModules();
    const third = await loadFreshStore();
    expect(third.getState().mode).toBe("chat");
  });

  it("persists and rehydrates the workflow mode", async () => {
    const first = await loadFreshStore();
    first.getState().setMode("workflow");

    vi.resetModules();
    const second = await loadFreshStore();
    expect(second.getState().mode).toBe("workflow");
  });
});
