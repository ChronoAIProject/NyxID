import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ChatActorControls } from "@/components/assistant/chat-actor-controls";
import { createChatActorProjection } from "@/lib/assistant/chat-actor-state";

describe("ChatActorControls state fence", () => {
  it("explains the version fence and disables pending approval controls", () => {
    const projection = createChatActorProjection("nyxid-chat-alpha");
    projection.pendingApproval = {
      approvalRequestId: "approval-alpha",
      toolName: "service.update",
    };

    render(
      <ChatActorControls
        projection={projection}
        disabled
        actionOverrides={new Map()}
        onResolveInput={vi.fn()}
        onResolveApproval={vi.fn()}
        onResolvePlan={vi.fn()}
        onStop={vi.fn()}
        onControlStep={vi.fn()}
        onActionProgress={vi.fn()}
        onBlockAction={vi.fn()}
        onResolveAction={vi.fn()}
      />,
    );

    expect(
      screen.getByText("Waiting for current state before controls can be used."),
    ).toHaveAttribute("role", "status");
    expect(screen.getByRole("button", { name: "Reject" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
  });
});
