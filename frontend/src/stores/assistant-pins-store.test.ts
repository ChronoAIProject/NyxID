import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createAssistantPinsStore, pinnedConversationsFirst, PIN_STORAGE_KEY } from "./assistant-pins-store";

describe("assistant conversation pins", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.restoreAllMocks());

  it("persists multiple pins across reload and removes only the unpinned chat", () => {
    const store = createAssistantPinsStore();
    store.getState().bindAccount("alice");
    store.getState().toggle("alice", "nyxa-old");
    store.getState().toggle("alice", "nyxa-older");
    const restored = createAssistantPinsStore();
    restored.getState().bindAccount("alice");
    expect(restored.getState().ids).toEqual(["nyxa-old", "nyxa-older"]);
    restored.getState().toggle("alice", "nyxa-old");
    expect(restored.getState().ids).toEqual(["nyxa-older"]);
  });

  it("clears persisted and in-memory pins on sign-out or account change", () => {
    const store = createAssistantPinsStore();
    store.getState().bindAccount("alice");
    store.getState().toggle("alice", "nyxa-old");
    store.getState().bindAccount("bob");
    expect(store.getState().ids).toEqual([]);
    expect(localStorage.getItem(PIN_STORAGE_KEY)).toBeNull();
    store.getState().toggle("alice", "nyxa-stale");
    expect(store.getState().ids).toEqual([]);
    store.getState().toggle("bob", "nyxa-new");
    store.getState().bindAccount(null);
    expect(store.getState().ids).toEqual([]);
    expect(localStorage.getItem(PIN_STORAGE_KEY)).toBeNull();
  });

  it("does not restore another account's saved pins on a fresh load", () => {
    localStorage.setItem(PIN_STORAGE_KEY, JSON.stringify({ version: 1, ownerUserId: "alice", ids: ["nyxa-old"] }));
    const store = createAssistantPinsStore();
    store.getState().bindAccount("bob");
    expect(store.getState().ids).toEqual([]);
    expect(localStorage.getItem(PIN_STORAGE_KEY)).toBeNull();
  });

  it("reports failed persistence without pretending the pin was saved, and allows retry", () => {
    const store = createAssistantPinsStore();
    store.getState().bindAccount("alice");
    const write = vi.spyOn(localStorage, "setItem").mockImplementation(() => { throw new Error("quota"); });
    store.getState().toggle("alice", "nyxa-old");
    expect(store.getState().ids).toEqual([]);
    expect(store.getState().error).toBe("save");
    write.mockRestore();
    store.getState().toggle("alice", "nyxa-old");
    expect(store.getState().ids).toEqual(["nyxa-old"]);
    expect(store.getState().error).toBeNull();
  });

  it("recovers safely from malformed storage and removes deleted conversation ids", () => {
    localStorage.setItem(PIN_STORAGE_KEY, "not json");
    const store = createAssistantPinsStore();
    store.getState().bindAccount("alice");
    expect(store.getState().error).toBe("load");
    store.getState().toggle("alice", "nyxa-old");
    store.getState().remove("alice", "nyxa-old");
    expect(store.getState().ids).toEqual([]);
    expect(localStorage.getItem(PIN_STORAGE_KEY)).toBeNull();
  });

  it("reports inaccessible storage and clears memory even if sign-out cleanup fails", () => {
    const store = createAssistantPinsStore();
    store.getState().bindAccount("alice");
    store.getState().toggle("alice", "nyxa-old");
    vi.spyOn(localStorage, "removeItem").mockImplementation(() => { throw new Error("blocked"); });
    store.getState().bindAccount(null);
    expect(store.getState().ids).toEqual([]);
    expect(store.getState().ownerUserId).toBeNull();
    expect(store.getState().error).toBe("clear");
    vi.spyOn(localStorage, "getItem").mockImplementation(() => { throw new Error("blocked"); });
    store.getState().bindAccount("bob");
    expect(store.getState().ids).toEqual([]);
    expect(store.getState().error).toBe("load");
  });

  it("keeps input order and object identity without mutating the transport snapshot", () => {
    const rows = Object.freeze([{ id: "new" }, { id: "old" }, { id: "older" }]);
    const sorted = pinnedConversationsFirst(rows, new Set(["older", "old", "other-agent"]));
    expect(sorted).toEqual([rows[1], rows[2], rows[0]]);
    expect(sorted[0]).toBe(rows[1]);
    expect(rows.map((row) => row.id)).toEqual(["new", "old", "older"]);
    expect(pinnedConversationsFirst(rows, new Set())).toEqual(rows);
  });
});
