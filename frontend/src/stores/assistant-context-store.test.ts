import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAssistantContextStore } from "./assistant-context-store";

function resetStore() {
  useAssistantContextStore.setState({
    ownerUserId: null,
    lastScreen: null,
    bindings: {},
  });
}

describe("useAssistantContextStore", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStore();
    vi.restoreAllMocks();
  });

  it("records a screen and binds its conversation", () => {
    const store = useAssistantContextStore.getState();
    store.recordScreen("user-1", "/keys");
    store.bindConversation("user-1", "/keys", "conversation-keys");

    expect(useAssistantContextStore.getState()).toMatchObject({
      ownerUserId: "user-1",
      lastScreen: "/keys",
      bindings: {
        "/keys": { conversationId: "conversation-keys" },
      },
    });
    expect(localStorage.getItem("nyxid.assistant_context")).toContain(
      "conversation-keys",
    );
  });

  it("resets prior-account state before applying a mutation", () => {
    const store = useAssistantContextStore.getState();
    store.recordScreen("user-1", "/keys");
    store.bindConversation("user-1", "/keys", "conversation-private");
    useAssistantContextStore.getState().recordScreen("user-2", "/nodes");

    expect(useAssistantContextStore.getState()).toMatchObject({
      ownerUserId: "user-2",
      lastScreen: "/nodes",
      bindings: {},
    });
  });

  it("keeps only the 50 most recently bound screens", () => {
    vi.spyOn(Date, "now").mockReturnValue(1_000);

    for (let index = 0; index < 51; index += 1) {
      useAssistantContextStore
        .getState()
        .bindConversation(
          "user-1",
          `/screen-${String(index)}`,
          `conversation-${String(index)}`,
        );
    }

    const { bindings } = useAssistantContextStore.getState();
    expect(Object.keys(bindings)).toHaveLength(50);
    expect(bindings["/screen-0"]).toBeUndefined();
    expect(bindings["/screen-50"]?.conversationId).toBe("conversation-50");
  });

  it("prunes bindings for server-deleted conversations", () => {
    const store = useAssistantContextStore.getState();
    store.bindConversation("user-1", "/keys", "conversation-keys");
    store.bindConversation("user-1", "/nodes", "conversation-nodes");
    useAssistantContextStore.getState().pruneBindings(["conversation-nodes"]);

    expect(useAssistantContextStore.getState().bindings).toEqual({
      "/nodes": expect.objectContaining({
        conversationId: "conversation-nodes",
      }),
    });
  });

  it("clears state and removes its persisted payload", () => {
    const store = useAssistantContextStore.getState();
    store.bindConversation("user-1", "/keys", "conversation-keys");
    useAssistantContextStore.getState().clear();

    expect(useAssistantContextStore.getState()).toMatchObject({
      ownerUserId: null,
      lastScreen: null,
      bindings: {},
    });
    expect(localStorage.getItem("nyxid.assistant_context")).toBeNull();
  });
});
