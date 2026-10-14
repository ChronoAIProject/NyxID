import { describe, expect, it } from "vitest";
import { assistantContextSchema } from "./assistant-context";

describe("assistantContextSchema", () => {
  it("parses a durable checkpoint with its scope", () => {
    const parsed = assistantContextSchema.parse({
      conversation_id: "nyxa-a",
      agent_id: "agent",
      binding_state: "reset",
      reset_reason: "turn_failed",
      pending_actions: [],
      checkpoint: {
        turn_id: "t2",
        status: "failed",
        error_code: "idle_timeout",
        goal: "Book the venue",
        completed_actions: [{ label: "mail__send", turn_id: "t2" }],
        failed_actions: [],
        last_successful_turn_id: "t1",
        updated_at: "2026-01-01T00:00:00Z",
      },
    });
    expect(parsed.checkpoint?.completed_actions[0]?.label).toBe("mail__send");
  });
  it("accepts a conversation without a checkpoint yet", () => {
    const parsed = assistantContextSchema.parse({
      conversation_id: "nyxa-a",
      binding_state: "bound",
      pending_actions: ["mcp.long_call"],
      checkpoint: null,
    });
    expect(parsed.checkpoint).toBeNull();
  });
});
