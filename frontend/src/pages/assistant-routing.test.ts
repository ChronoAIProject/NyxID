import { describe, expect, it } from "vitest";
import { assistantChatSurface } from "@/lib/assistant/conversation-ids";

describe("assistantChatSurface", () => {
  it("routes flag-enabled drafts and direct ids through the Direct seam", () => {
    expect(
      assistantChatSurface({
        directEnabled: true,
        drafting: true,
        selectedConversationId: "nyxid-chat-existing",
      }),
    ).toBe("direct");
    expect(
      assistantChatSurface({
        directEnabled: true,
        drafting: false,
        selectedConversationId: "direct-local",
      }),
    ).toBe("direct");
  });

  it("keeps persisted actor ids on the canonical pipeline", () => {
    expect(
      assistantChatSurface({
        directEnabled: true,
        drafting: false,
        selectedConversationId: "nyxid-chat-existing",
      }),
    ).toBe("actor");
    expect(
      assistantChatSurface({
        directEnabled: true,
        drafting: false,
        selectedConversationId: "chatc-legacy",
      }),
    ).toBe("actor");
    expect(
      assistantChatSurface({
        directEnabled: false,
        drafting: false,
      }),
    ).toBe("actor");
  });
});
