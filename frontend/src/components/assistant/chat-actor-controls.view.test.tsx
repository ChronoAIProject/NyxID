import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ChatActorControls } from "@/components/assistant/chat-actor-controls";
import {
  createChatActorProjection,
  decodeActorFrame,
  reduceActorFrame,
  updateApprovalDecisionSubmission,
} from "@/lib/assistant/chat-actor-state";

function pendingApprovalProjection() {
  const projection = createChatActorProjection("nyxid-chat-alpha");
  projection.stateVersion = 7;
  return reduceActorFrame(
    projection,
    decodeActorFrame({
      sequence: 1,
      custom: {
        name: "nyxid.approval.request",
        payload: {
          approvalRequestId: "approval-alpha",
          toolName: "service.update",
          serviceSlug: "github",
          expiresAt: "2099-08-08T01:00:00Z",
          message: "Update the GitHub service.",
        },
      },
    }),
  );
}

const callbacks = {
  actionOverrides: new Map(),
  onResolveInput: vi.fn(),
  onResolveApproval: vi.fn(),
  onStop: vi.fn(),
  onControlStep: vi.fn(),
  onActionProgress: vi.fn(),
  onBlockAction: vi.fn(),
  onResolveAction: vi.fn(),
};

describe("ChatActorControls state fence", () => {
  it("explains the version fence and disables pending approval controls", () => {
    const projection = pendingApprovalProjection();

    render(
      <ChatActorControls
        projection={projection}
        disabled
        {...callbacks}
      />,
    );

    expect(
      screen.getByText("Waiting for current state before controls can be used."),
    ).toHaveAttribute("role", "status");
    expect(screen.getByRole("button", { name: "Deny" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Approve and send" })).toBeDisabled();
  });

  it("keeps the approval card through accepted and committed states", () => {
    const pending = pendingApprovalProjection();
    const { rerender } = render(
      <ChatActorControls projection={pending} disabled={false} {...callbacks} />,
    );
    expect(screen.getByRole("button", { name: "Approve and send" })).toBeEnabled();

    const submitted = updateApprovalDecisionSubmission(
      pending,
      "approval-alpha",
      "approved",
    );
    rerender(
      <ChatActorControls projection={submitted} disabled={false} {...callbacks} />,
    );
    expect(screen.getByText("Decision sent")).toBeVisible();

    const committed = reduceActorFrame(
      submitted,
      decodeActorFrame({
        sequence: 2,
        custom: {
          name: "nyxid.approval.changed",
          payload: {
            approvalRequestId: "approval-alpha",
            outcome: "accepted",
            approved: true,
          },
        },
      }),
    );
    rerender(
      <ChatActorControls projection={committed} disabled={false} {...callbacks} />,
    );
    expect(screen.getByText("Approved and sent")).toBeVisible();
  });
});
