import { describe, expect, it } from "vitest";
import {
  assistantGroupMessageSchema,
  nyxAgentMessageSchema,
} from "@/schemas/assistant-nyxagent";
import { UI_EVENT_NAMES } from "@/lib/telemetry-schema";

describe("assistant feedback contracts", () => {
  it("decodes persisted group feedback", () => {
    const value = assistantGroupMessageSchema.parse({
      id: "m",
      group_id: "g",
      seq: 1,
      role: "agent",
      text: "ok",
      created_at: new Date().toISOString(),
      activities: [],
      attachments: [],
      feedback: { vote: "down", comment: "short" },
    });
    expect(value.feedback?.vote).toBe("down");
  });
  it("preserves conversation feedback on reload", () => {
    const value = nyxAgentMessageSchema.parse({
      id: "m",
      seq: 1,
      turn_id: "t",
      role: "assistant",
      text: "ok",
      status: "completed",
      error_code: null,
      created_at: new Date().toISOString(),
      activities: [],
      attachments: [],
      feedback: {
        vote: "up",
        comment: "yes",
        updated_at: new Date().toISOString(),
      },
    });
    expect(value.feedback?.vote).toBe("up");
  });
  it("keeps feedback telemetry on the closed allowlist", () => {
    expect(UI_EVENT_NAMES).toContain("ui.assistant_feedback_submitted");
  });
});
