import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AssistantMessage } from "@/types/assistant";
import { ChatThread } from "./chat-thread";

vi.mock("@/components/assistant/blocks/action-card", () => ({
  ActionCard: ({
    block,
  }: {
    readonly block: { readonly action_request_id: string };
  }) => <div data-testid="action-card-dispatch">{block.action_request_id}</div>,
}));

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

  it("shows the thinking indicator while a turn awaits its first content", () => {
    render(
      <ChatThread
        messages={[
          message({
            role: "user",
            blocks: [{ type: "text", block_id: "text-1", text: "Hi" }],
          }),
        ]}
        thinking
        onDecideApproval={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("status", { name: "Assistant is thinking" }),
    ).toBeInTheDocument();
  });

  it("hides the thinking indicator once assistant content streams", () => {
    render(
      <ChatThread
        messages={[
          message({
            blocks: [{ type: "text", block_id: "text-1", text: "Answer" }],
          }),
        ]}
        onDecideApproval={vi.fn()}
      />,
    );

    expect(
      screen.queryByRole("status", { name: "Assistant is thinking" }),
    ).not.toBeInTheDocument();
  });

  it("dispatches action_card blocks to the rich action renderer", () => {
    render(
      <ChatThread
        messages={[
          message({
            blocks: [
              {
                type: "action_card",
                block_id: "action-card-1",
                action: "service.connect",
                action_request_id: "act-1",
                origin_turn_id: "turn-1",
                params: {
                  variant: "catalog",
                  service_slug: "api-github",
                  requested_scopes: ["repo"],
                  via_node_id: null,
                  target_org_id: null,
                },
                status: "pending",
                outcome_note: "",
              },
            ],
          }),
        ]}
        onDecideApproval={vi.fn()}
      />,
    );

    expect(screen.getByTestId("action-card-dispatch")).toHaveTextContent(
      "act-1",
    );
  });
});
