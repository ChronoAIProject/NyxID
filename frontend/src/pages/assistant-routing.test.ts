import { describe, expect, it } from "vitest";
import { assistantChatSurface } from "@/lib/assistant/conversation-ids";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { parseSkillGallerySearch } from "@/lib/assistant/skill-templates";

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

describe("skill template links", () => {
  const skill = "research-paper-writing-rigor";

  it("accepts a known slug with fixed starter content", () => {
    expect(parseAssistantSearch({ skill, prompt: "untrusted prompt" })).toEqual({ skill });
    expect(parseAssistantSearch({ skill: "unknown" })).toEqual({});
    expect(parseAssistantSearch({ skill: [skill, skill] })).toEqual({});
  });

  it.each(["c", "draft", "agent", "g", "card", "proposal", "details", "connection", "panel"])(
    "ignores a template when the %s selector is present",
    (key) => {
      expect(parseAssistantSearch({ skill, [key]: "somewhere-else" }).skill).toBeUndefined();
    },
  );

  it("retains only a known gallery slug and a bounded invite code", () => {
    expect(parseSkillGallerySearch({ skill, code: "TEST-1234", return_to: "https://example.com" }))
      .toEqual({ skill, code: "TEST-1234" });
    expect(parseSkillGallerySearch({ skill: "unknown", code: "a".repeat(65) })).toEqual({});
  });
});
