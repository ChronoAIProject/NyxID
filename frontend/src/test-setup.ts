import "@testing-library/jest-dom/vitest";

// happy-dom 20.x ships a stub `localStorage` without the Storage API
// methods (it expects a `--localstorage-file` path to enable full
// persistence). We back it with a plain in-memory map so the tests
// and state persist middleware have a real, working localStorage.
const store = new Map<string, string>();
const localStorageMock: Storage = {
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
  value: localStorageMock,
  configurable: true,
  writable: true,
});
