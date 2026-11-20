import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { ApprovalCardContentBlock } from "@/types/assistant";
import { approvalRequestToBlock } from "@/lib/assistant/chat-approval-presentation";
import { CardLanguageContext } from "../card-language";
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
  it("localizes only client-authored approval templates in the chat language", async () => {
    await appI18n.loadLanguages("zh-CN");
    const request = { approvalRequestId: "test", toolName: "send_digest", expiresAt: approval().expires_at };
    const show = (fields: Record<string, unknown>) => (
      <CardLanguageContext.Provider value="zh-CN">
        <ApprovalCard block={approvalRequestToBlock({ ...request, ...fields }, 1)} onDecide={vi.fn()} />
      </CardLanguageContext.Provider>
    );
    const { rerender } = render(show({}));
    expect(screen.getByText("助手请求运行 send_digest。")).toBeVisible();
    rerender(show({ toolName: "" }));
    expect(screen.getByText("助手需要你批准才能继续。")).toBeVisible();
    rerender(show({ presentation: { action: "post", target: "#digest" } }));
    expect(screen.getByText("post（目标：#digest）")).toBeVisible();
    rerender(show({ body: "The assistant wants to run send_digest." }));
    expect(screen.getByText("The assistant wants to run send_digest.")).toBeVisible();
    expect(appI18n.language).toBe("en");
  });

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
    expect(screen.getByText("Approve within 840 seconds")).toBeInTheDocument();
  });

  it.each([
    ["zh-CN", "请在 10 秒内批准"],
    ["zh-TW", "請在 10 秒內核准"],
  ] as const)("renders the remaining seconds in %s", async (language, label) => {
    await appI18n.loadLanguages(language);
    render(
      <CardLanguageContext.Provider value={language}>
        <ApprovalCard block={{ ...approval(), expires_at: new Date(NOW + 10_000).toISOString() }} onDecide={vi.fn()} />
      </CardLanguageContext.Provider>,
    );
    expect(screen.getByText(label)).toBeVisible();
  });

  it("counts down from the stored deadline and requires a fresh request after expiry", () => {
    const decide = vi.fn();
    const requestAgain = vi.fn();
    const block = { ...approval(), expires_at: new Date(NOW + 80_000).toISOString() };
    render(<ApprovalCard block={block} onDecide={decide} onRequestAgain={requestAgain} />);
    act(() => { vi.advanceTimersByTime(70_000); });
    expect(screen.getByText("Approve within 10 seconds")).toBeVisible();
    expect(screen.getByRole("button", { name: "Approve and send" })).toBeEnabled();
    act(() => { vi.advanceTimersByTime(15_000); });
    expect(screen.getByText("Request expired")).toBeVisible();
    expect(screen.getByText("Nothing was sent.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Approve and send" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Ask for a new confirmation" }));
    expect(requestAgain).toHaveBeenCalledOnce();
    expect(decide).not.toHaveBeenCalled();
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
