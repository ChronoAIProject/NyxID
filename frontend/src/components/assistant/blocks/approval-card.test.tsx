import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { ApprovalCardContentBlock } from "@/types/assistant";
import { ApprovalCard } from "./approval-card";

const NOW = Date.parse("2026-07-16T04:00:00.000Z");

function approval(
  decision: ApprovalCardContentBlock["decision"] = null,
): ApprovalCardContentBlock {
  return {
    type: "approval_card",
    block_id: "approval-1",
    approval_request_id: "request-1",
    body: "Post the digest to #payments-oncall.",
    service_slug: "lark-bot",
    agent_key_prefix: "nyxid_ag_...7f3d",
    approval_mode: "per_request",
    grant_duration_sec: null,
    expires_at: new Date(NOW + 14 * 60_000).toISOString(),
    decision,
    decision_channel: decision ? "web" : null,
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("ApprovalCard", () => {
  it("approves once and throttles a repeated click", async () => {
    const onDecide = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(
      <ApprovalCard block={approval()} onDecide={onDecide} />,
    );

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Approve and send" }));
      fireEvent.click(screen.getByRole("button", { name: "Approve and send" }));
    });
    expect(onDecide).toHaveBeenCalledTimes(1);
    expect(onDecide).toHaveBeenCalledWith(true);

    rerender(<ApprovalCard block={approval("approved")} onDecide={onDecide} />);
    expect(screen.getByText("Approved and sent")).toBeInTheDocument();
    expect(screen.getByText("via web")).toBeInTheDocument();
  });

  it("sends the deny branch and renders its terminal state", async () => {
    const onDecide = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(
      <ApprovalCard block={approval()} onDecide={onDecide} />,
    );

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    });
    expect(onDecide).toHaveBeenCalledWith(false);

    rerender(<ApprovalCard block={approval("denied")} onDecide={onDecide} />);
    expect(screen.getByText("Request denied")).toBeInTheDocument();
    expect(screen.getByText("Nothing was sent.")).toBeInTheDocument();
  });

  it("renders the structured scope row on a pending card", () => {
    render(<ApprovalCard block={approval()} onDecide={vi.fn()} />);
    const card = screen
      .getByRole("heading", { name: "Approval required" })
      .closest("section");
    expect(card).toHaveClass("border-border", "bg-card");
    expect(card?.className).not.toContain("warning");
    expect(card?.querySelector('[class*="warning"]')).toBeNull();
    expect(card?.firstElementChild).toHaveClass("bg-info");
    expect(screen.getByText("Pending")).toBeInTheDocument();
    expect(screen.getByText("Scope")).toBeInTheDocument();
    expect(screen.getByText("lark-bot")).toBeInTheDocument();
    expect(screen.getByText("nyxid_ag_...7f3d")).toBeInTheDocument();
    expect(screen.getByText("per-request approval")).toBeInTheDocument();
    expect(screen.getByText(/expires in 14 min/)).toBeInTheDocument();
  });

  it("explains what a grant-mode approval allows", () => {
    render(
      <ApprovalCard
        block={{
          ...approval(),
          approval_mode: "grant",
          grant_duration_sec: 3600,
        }}
        onDecide={vi.fn()}
      />,
    );
    expect(screen.getByText("grant · 1 h")).toBeInTheDocument();
    expect(
      screen.getByText(/repeat lark-bot writes for the next 1 h/),
    ).toBeInTheDocument();
  });

  it("renders expired and cancelled terminal states distinctly", () => {
    const { rerender } = render(
      <TooltipProvider>
        <ApprovalCard block={approval("expired")} onDecide={vi.fn()} />
      </TooltipProvider>,
    );
    expect(screen.getByText("Request expired")).toBeInTheDocument();
    expect(screen.getByText("Nothing was sent.")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /request again/i }),
    ).toBeInTheDocument();

    rerender(
      <TooltipProvider>
        <ApprovalCard block={approval("cancelled")} onDecide={vi.fn()} />
      </TooltipProvider>,
    );
    expect(screen.getByText("Request cancelled")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /request again/i }),
    ).not.toBeInTheDocument();
  });
});
