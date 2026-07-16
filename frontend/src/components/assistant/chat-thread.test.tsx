import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AssistantMessage } from "@/types/assistant";
import { ChatThread } from "./chat-thread";

function message(overrides: Partial<AssistantMessage>): AssistantMessage {
  return {
    id: "message-1",
    role: "assistant",
    schema_version: 1,
    blocks: [],
    created_at: "2026-07-16T04:00:00.000Z",
    ...overrides,
  };
}

describe("ChatThread", () => {
  it("renders an unsupported shell for an unknown block type", () => {
    const unknown = message({
      blocks: [
        {
          type: "future_card",
          block_id: "future-1",
          payload: "opaque",
        },
      ] as unknown as AssistantMessage["blocks"],
    });
    render(<ChatThread messages={[unknown]} onDecideApproval={vi.fn()} />);

    expect(
      screen.getByText("Unsupported assistant content"),
    ).toBeInTheDocument();
  });

  it("renders an unsupported shell for a newer message schema", () => {
    render(
      <ChatThread
        messages={[
          message({
            schema_version: 2,
            blocks: [{ type: "text", block_id: "text-1", text: "Hidden" }],
          }),
        ]}
        onDecideApproval={vi.fn()}
      />,
    );

    expect(
      screen.getByText("Unsupported assistant content"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Hidden")).not.toBeInTheDocument();
  });
});
