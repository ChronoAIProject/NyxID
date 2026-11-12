import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { ApprovalCardContentBlock } from "@/types/assistant";
import { ApprovalCard } from "./approval-card";
import { appI18n } from "@/i18n";

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

afterEach(async () => {
  cleanup();
  vi.useRealTimers();
  await appI18n.changeLanguage("en");
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
    expect(screen.getByText("Approved")).toBeInTheDocument();
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

  it("renders the purple pending treatment without a top highlight", () => {
    render(<ApprovalCard block={approval()} onDecide={vi.fn()} />);
    const card = screen
      .getByRole("heading", { name: "Approval required" })
      .closest("section");
    expect(card).toHaveClass("border-border", "bg-card");
    expect(card?.className).not.toContain("warning");
    expect(card?.querySelector('[class*="warning"]')).toBeNull();
    expect(card?.firstElementChild).toHaveClass("flex", "border-b");
    expect(screen.getByText("Pending")).toHaveClass("text-nyx-secondary-400");
    expect(screen.getByText("Scope")).toBeInTheDocument();
    expect(screen.getByText("lark-bot")).toBeInTheDocument();
    expect(screen.getByText("nyxid_ag_...7f3d")).toBeInTheDocument();
    expect(screen.getByText("per-request approval")).toBeInTheDocument();
    expect(screen.getByText(/expires in 14 min/)).toBeInTheDocument();
  });

  it("keeps grant approvals purple and explains the broader approval", () => {
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
    expect(screen.getByText("Pending")).toHaveClass(
      "text-nyx-secondary-400",
    );
    expect(
      screen
        .getByRole("heading", { name: "Approval required" })
        .closest("section")
        ?.querySelector('[class*="warning"]'),
    ).toBeNull();
    expect(screen.getByText("grant · 1 h")).toBeInTheDocument();
    expect(
      screen.getByText(/repeat lark-bot writes for the next 1 h/),
    ).toBeInTheDocument();
  });

  it("renders expired and cancelled terminal states distinctly", () => {
    const { rerender } = render(
      <TooltipProvider>
        <ApprovalCard block={approval("expired")} onDecide={vi.fn()} onRequestAgain={vi.fn()} />
      </TooltipProvider>,
    );
    expect(screen.getByText("Request expired")).toBeInTheDocument();
    expect(screen.getByText("Nothing was sent.")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /new confirmation/i }),
    ).toBeInTheDocument();

    rerender(
      <TooltipProvider>
        <ApprovalCard block={approval("cancelled")} onDecide={vi.fn()} />
      </TooltipProvider>,
    );
    expect(screen.getByText("Request cancelled")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /new confirmation/i }),
    ).not.toBeInTheDocument();
  });
});


it("2072 shows local expiry, a working review link and a real fresh-request action", () => {
  const decide = vi.fn();
  const request = vi.fn();
  render(<ApprovalCard block={{ ...approval(), expires_at: new Date(NOW + 1).toISOString() }} onDecide={decide} onRequestAgain={request} reviewPath="/assistant/approvals" />);
  act(() => { vi.advanceTimersByTime(30_000); });
  expect(screen.getByText("Request expired")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Approve and send" })).toBeNull();
  expect(screen.getByRole("link", { name: "Open Approvals" })).toHaveAttribute("href", "/assistant/approvals");
  fireEvent.click(screen.getByRole("button", { name: "Ask for a new confirmation" }));
  expect(request).toHaveBeenCalledTimes(1);
  expect(decide).not.toHaveBeenCalled();
});
it("2072 never offers an inert retry when no consumer supports it", () => {
  render(<ApprovalCard block={approval("expired")} onDecide={vi.fn()} />);
  expect(screen.queryByRole("button")).toBeNull();
});

it.each([
  ["approved", "Approved", "已批准", "已核准"],
  ["denied", "Request denied", "请求被拒绝", "請求被拒絕"],
  ["expired", "Request expired", "请求已过期", "請求已過期"],
  ["cancelled", "Request cancelled", "请求已取消", "請求已取消"],
] as const)("translates a mounted %s title without changing the server body", async (decision, english, simplified, traditional) => {
  render(<ApprovalCard block={approval(decision)} onDecide={vi.fn()} />);
  expect(screen.getByText(english)).toBeVisible();
  for (const [language, title] of [["zh-CN", simplified], ["zh-TW", traditional], ["en", english]]) {
    await act(async () => { await appI18n.changeLanguage(language); });
    expect(screen.getByText(title!)).toBeVisible();
    expect(screen.getByText(approval().body)).toBeVisible();
  }
});

it.each([
  [1800, "30 min", "30 分钟"],
  [3600, "1 h", "1 小时"],
  [86400, "1 d", "1 天"],
] as const)("translates the countdown and %s-second grant explanation", async (seconds, english, chinese) => {
  render(<ApprovalCard block={{ ...approval(), approval_mode: "grant", grant_duration_sec: seconds }} onDecide={vi.fn()} />);
  expect(screen.getByText(`grant · ${english}`)).toBeVisible();
  expect(screen.getByText("expires in 14 min")).toBeVisible();
  await act(async () => { await appI18n.changeLanguage("zh-CN"); });
  expect(screen.getByText(`授权 · ${chinese}`)).toBeVisible();
  expect(screen.getByText("14 分钟后过期")).toBeVisible();
  expect(screen.getByText(`批准后，接下来的 ${chinese} 内可重复向 lark-bot 写入，无需再次询问。`)).toBeVisible();
});

it.each([
  ["approved", "Waiting for the committed approved decision.", "正在等待批准决定提交完成。"],
  ["denied", "Waiting for the committed denied decision.", "正在等待拒绝决定提交完成。"],
] as const)("translates a submitted %s decision as a whole phrase", async (decision_submission, english, chinese) => {
  render(<ApprovalCard block={{ ...approval(), decision_submission }} onDecide={vi.fn()} />);
  expect(screen.getByText(english)).toBeVisible();
  await act(async () => { await appI18n.changeLanguage("zh-CN"); });
  expect(screen.getByText(chinese)).toBeVisible();
});
