import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAssistantContextStore } from "./assistant-context-store";

function resetStore() {
  useAssistantContextStore.setState({
    ownerUserId: null,
    lastScreen: null,
  });
}

describe("useAssistantContextStore", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStore();
    vi.restoreAllMocks();
  });

  it("records the last screen without persisting a conversation selection", () => {
    const store = useAssistantContextStore.getState();
    store.recordScreen("user-1", "/keys");

    expect(useAssistantContextStore.getState()).toMatchObject({
      ownerUserId: "user-1",
      lastScreen: "/keys",
    });
    expect(localStorage.getItem("nyxid.assistant_context")).not.toContain(
      "bindings",
    );
  });

  it("resets prior-account state before applying a mutation", () => {
    const store = useAssistantContextStore.getState();
    store.recordScreen("user-1", "/keys");
    useAssistantContextStore.getState().recordScreen("user-2", "/nodes");

    expect(useAssistantContextStore.getState()).toMatchObject({
      ownerUserId: "user-2",
      lastScreen: "/nodes",
    });
  });

  it("migrates persisted v1 state without its conversation bindings", async () => {
    localStorage.setItem(
      "nyxid.assistant_context",
      JSON.stringify({
        state: {
          ownerUserId: "user-1",
          lastScreen: "/keys",
          bindings: {
            "/keys": {
              conversationId: "conversation-private",
              updatedAt: 1,
            },
          },
        },
        version: 1,
      }),
    );

    await useAssistantContextStore.persist.rehydrate();

    const state = useAssistantContextStore.getState();
    expect(state).toMatchObject({
      ownerUserId: "user-1",
      lastScreen: "/keys",
    });
    expect("bindings" in state).toBe(false);
    expect(localStorage.getItem("nyxid.assistant_context")).not.toContain(
      "conversation-private",
    );
  });

  it("clears state and removes its persisted payload", () => {
    const store = useAssistantContextStore.getState();
    store.recordScreen("user-1", "/keys");
    useAssistantContextStore.getState().clear();

    expect(useAssistantContextStore.getState()).toMatchObject({
      ownerUserId: null,
      lastScreen: null,
    });
    expect(localStorage.getItem("nyxid.assistant_context")).toBeNull();
  });
});
