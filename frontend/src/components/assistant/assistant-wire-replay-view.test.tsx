import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AssistantWireLogExchange } from "@/schemas/assistant-wire-log";
import {
  AssistantWireReplayView,
  replayAssistantWireExchange,
} from "./assistant-wire-replay-view";

function exchange(text: string, truncated = false): AssistantWireLogExchange {
  return {
    id: "exchange-1",
    ts: 1,
    kind: "header",
    status: 200,
    conversationId: "nyxid-chat-replay",
    wireLogId: "wire-1",
    label: "POST /assistant/chat",
    capture: {
      state: "settled",
      outcome: "complete",
      wireOutcome: "complete",
      body: { text, bytes: text.length, truncated },
    },
  };
}

describe("AssistantWireReplayView", () => {
  it("replays mixed line endings through the live normalizer and keeps actor facts inert", () => {
    const text = [
      'data: {"runStarted":{"actorId":"nyxid-chat-replay","runId":"turn-1"}}\r\n\r\n',
      'data: {"custom":{"name":"aevatar.llm.reasoning","payload":{"delta":"Checking."}}}\n\n',
      'data: {"sequence":1,"custom":{"name":"nyxid.approval.request","payload":{"approvalRequestId":"approval-1","toolName":"deploy"}}}\r\r',
      "data: {malformed\n\n",
      'data: {"textMessageContent":{"messageId":"message-1","delta":"Hello from replay"}}\n\n',
      'data: {"runFinished":{"runId":"turn-1","result":{"output":"Hello from replay"}}}\n\n',
      "data: [DONE]\n\n",
    ].join("");

    const replay = replayAssistantWireExchange(exchange(text));
    expect(replay).toMatchObject({
      partial: false,
      message: { content: "Hello from replay", status: "complete" },
      actorProjection: {
        pendingApproval: { approvalRequestId: "approval-1" },
      },
    });
    render(<AssistantWireReplayView exchange={exchange(text)} />);
    expect(screen.getByText("Hello from replay")).toBeVisible();
    expect(
      screen.getByRole("region", { name: "Actor facts (diagnostic only)" }),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
  });

  it("marks truncated or unterminated captures as partial", () => {
    const replay = replayAssistantWireExchange(
      exchange(
        'data: {"runStarted":{"actorId":"nyxid-chat-replay","runId":"turn-1"}}\n\n',
        true,
      ),
    );
    expect(replay?.partial).toBe(true);
  });
});
