import { describe, expect, it } from "vitest";
import type { Conversation } from "@/types/assistant";
import { resolveAssistantConversationId } from "./conversation-resolution";

const conversations: Conversation[] = [
  {
    id: "newest",
    title: "Newest",
    created_at: "2026-07-29T03:00:00.000Z",
    last_message_at: "2026-07-29T03:00:00.000Z",
  },
  {
    id: "bound",
    title: "Bound",
    created_at: "2026-07-28T03:00:00.000Z",
    last_message_at: "2026-07-28T03:00:00.000Z",
  },
  {
    id: "deep-link",
    title: "Deep link",
    created_at: "2026-07-27T03:00:00.000Z",
    last_message_at: "2026-07-27T03:00:00.000Z",
  },
];

describe("resolveAssistantConversationId", () => {
  it("prioritizes an explicit deep link over a binding and newest", () => {
    expect(
      resolveAssistantConversationId({
        explicitConversationId: "deep-link",
        boundConversationId: "bound",
        entryScreen: "/keys",
        conversationsResolved: true,
        conversations,
      }),
    ).toBe("deep-link");
  });

  it("uses an existing screen binding before newest", () => {
    expect(
      resolveAssistantConversationId({
        boundConversationId: "bound",
        entryScreen: "/keys",
        conversationsResolved: true,
        conversations,
      }),
    ).toBe("bound");
  });

  it("keeps an unbound recorded screen blank instead of selecting newest", () => {
    expect(
      resolveAssistantConversationId({
        entryScreen: "/nodes",
        conversationsResolved: true,
        conversations,
      }),
    ).toBeUndefined();
  });

  it("keeps a recorded screen blank when its binding no longer exists", () => {
    expect(
      resolveAssistantConversationId({
        boundConversationId: "deleted",
        entryScreen: "/keys",
        conversationsResolved: true,
        conversations,
      }),
    ).toBeUndefined();
  });

  it("falls back to newest only when there is no recorded entry screen", () => {
    expect(
      resolveAssistantConversationId({
        entryScreen: null,
        conversationsResolved: true,
        conversations,
      }),
    ).toBe("newest");
  });

  it("honors an explicit deep link while the conversation list is unresolved", () => {
    expect(
      resolveAssistantConversationId({
        explicitConversationId: "not-loaded-yet",
        entryScreen: "/keys",
        conversationsResolved: false,
        conversations: [],
      }),
    ).toBe("not-loaded-yet");
  });

  it("drops a stale explicit id after resolution and falls through", () => {
    expect(
      resolveAssistantConversationId({
        explicitConversationId: "deleted",
        boundConversationId: "bound",
        entryScreen: "/keys",
        conversationsResolved: true,
        conversations,
      }),
    ).toBe("bound");
  });
});
