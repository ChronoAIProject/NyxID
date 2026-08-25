import { describe, expect, it } from "vitest";
import {
  hydrateStoredMessages,
  resolveStoredConversationStatus,
  trimChatTitle,
} from "@/lib/assistant/chat-session-state";
import type { StoredChatMessage } from "@/lib/assistant/chat-types";

const COMPLETED: StoredChatMessage = {
  id: "assistant-1",
  turnId: "turn-1",
  role: "assistant",
  content: "Settled answer",
  timestamp: 1,
  status: "completed",
};

describe("chat session state", () => {
  it("preserves turnId and treats the production completed status as settled", () => {
    expect(hydrateStoredMessages([COMPLETED])).toEqual([
      expect.objectContaining({
        turnId: "turn-1",
        status: "complete",
        content: "Settled answer",
      }),
    ]);
    expect(resolveStoredConversationStatus([COMPLETED])).toBe("completed_text");
  });

  it("uses the console first-turn title normalization", () => {
    expect(trimChatTitle("  inspect   the audit trail ")).toBe(
      "inspect the audit trail",
    );
    expect(trimChatTitle("x".repeat(80))).toBe(`${"x".repeat(57)}...`);
  });
});
