import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAssistantDraftStore } from "./assistant-draft-store";

function resetStore() {
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
}

describe("useAssistantDraftStore", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStore();
    vi.restoreAllMocks();
  });

  it("saves exact text and deletes blank drafts", () => {
    const store = useAssistantDraftStore.getState();
    store.saveDraft("user-1", "conv:one", "  unfinished question  ");
    expect(useAssistantDraftStore.getState().getDraft("conv:one")).toBe(
      "  unfinished question  ",
    );

    useAssistantDraftStore.getState().saveDraft("user-1", "conv:one", "  ");
    expect(useAssistantDraftStore.getState().getDraft("conv:one")).toBe("");
  });

  it("clears one draft without affecting the others", () => {
    const store = useAssistantDraftStore.getState();
    store.saveDraft("user-1", "conv:one", "One");
    store.saveDraft("user-1", "screen:/keys", "New keys chat");
    useAssistantDraftStore.getState().clearDraft("user-1", "conv:one");

    expect(useAssistantDraftStore.getState().drafts).toEqual({
      "screen:/keys": expect.objectContaining({ text: "New keys chat" }),
    });
  });

  it("resets prior-account drafts before applying a mutation", () => {
    const store = useAssistantDraftStore.getState();
    store.saveDraft("user-1", "conv:private", "Private draft");
    useAssistantDraftStore
      .getState()
      .saveDraft("user-2", "conv:second", "Second account");

    expect(useAssistantDraftStore.getState()).toMatchObject({
      ownerUserId: "user-2",
      drafts: {
        "conv:second": { text: "Second account" },
      },
    });
    expect(
      useAssistantDraftStore.getState().drafts["conv:private"],
    ).toBeUndefined();
  });

  it("keeps only the 50 most recently written drafts", () => {
    vi.spyOn(Date, "now").mockReturnValue(1_000);

    for (let index = 0; index < 51; index += 1) {
      useAssistantDraftStore
        .getState()
        .saveDraft("user-1", `conv:${String(index)}`, `Draft ${String(index)}`);
    }

    const { drafts } = useAssistantDraftStore.getState();
    expect(Object.keys(drafts)).toHaveLength(50);
    expect(drafts["conv:0"]).toBeUndefined();
    expect(drafts["conv:50"]?.text).toBe("Draft 50");
  });

  it("prunes deleted conversation drafts but retains screen drafts", () => {
    const store = useAssistantDraftStore.getState();
    store.saveDraft("user-1", "conv:keep", "Keep");
    store.saveDraft("user-1", "conv:deleted", "Delete");
    store.saveDraft("user-1", "screen:/nodes", "Uncreated");
    useAssistantDraftStore.getState().pruneConversationDrafts(["keep"]);

    expect(useAssistantDraftStore.getState().drafts).toEqual({
      "conv:keep": expect.objectContaining({ text: "Keep" }),
      "screen:/nodes": expect.objectContaining({ text: "Uncreated" }),
    });
  });

  it("clears state and removes its persisted payload", () => {
    useAssistantDraftStore.getState().saveDraft("user-1", "conv:one", "Draft");
    useAssistantDraftStore.getState().clear();

    expect(useAssistantDraftStore.getState()).toMatchObject({
      ownerUserId: null,
      drafts: {},
    });
    expect(localStorage.getItem("nyxid.assistant_drafts")).toBeNull();
  });
});
