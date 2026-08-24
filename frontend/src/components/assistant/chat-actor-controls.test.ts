import { describe, expect, it } from "vitest";
import { actionSummaryBlock } from "@/lib/assistant/chat-action-presentation";
import type { ChatActionSummary } from "@/lib/assistant/chat-actor-state";

describe("chat actor action projection", () => {
  it("renders a recovered reload summary as unsupported without re-parsing", () => {
    const summary: ChatActionSummary = {
      schemaVersion: 5,
      actorId: "nyxid-chat-alpha",
      originTurnId: "turn-alpha",
      taskId: "task-alpha",
      stepId: "step-alpha",
      actionRequestId: "action-alpha",
      action: "future.action",
      request: null,
      supported: false,
      recovered: true,
      reports: [],
      postconditionResult: null,
    };
    expect(actionSummaryBlock(summary)).toMatchObject({
      action_request_id: "action-alpha",
      origin_turn_id: "turn-alpha",
      actor_id: "nyxid-chat-alpha",
      params: { variant: "unknown" },
      status: "unsupported",
    });
  });
});
