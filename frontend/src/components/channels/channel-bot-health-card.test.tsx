import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { ChannelBotHealthCard } from "./channel-bot-health-card";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  api: { get: mockGet, post: mockPost },
}));
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn() },
}));

function health(overrides: Record<string, unknown> = {}) {
  return {
    id: "bot-1",
    platform: "x",
    label: "Support",
    platform_bot_username: "support",
    user_id: "owner-1",
    mode: "webhook",
    status: "active",
    webhook_registered: true,
    poll_error_count: 0,
    last_polled_at: null,
    next_poll_at: null,
    error: null,
    recommended_action: "resubscribe",
    recommended_action_text:
      "The X webhook subscription is missing or incomplete. Register it again.",
    can_write: true,
    diagnostics: {
      last_inbound_at: "2026-10-12T08:00:00Z",
      last_inbound_source: "webhook",
      last_outbound_at: null,
      last_outbound_error: {
        code: "10005",
        reason: "rate_limited",
        at: "2026-10-12T09:00:00Z",
      },
      last_webhook_at: "2026-10-12T08:00:00Z",
      last_webhook_error: null,
      subscription_state: {
        registered: false,
        checked_at: "2026-10-12T09:30:00Z",
        last_error: "subscription_missing",
      },
      consecutive_errors: 2,
      failure_reason: "subscription_missing",
    },
    ...overrides,
  };
}

function renderCard() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ChannelBotHealthCard botId="bot-1" />
    </QueryClientProvider>,
  );
}

beforeEach(async () => {
  vi.clearAllMocks();
  await appI18n.changeLanguage("en");
});
afterEach(async () => {
  await act(async () => { await appI18n.changeLanguage("en"); });
});

describe("ChannelBotHealthCard", () => {
  it("shows a formatting fallback without claiming that delivery succeeded", async () => {
    const result = health({ platform: "telegram" });
    mockGet.mockResolvedValue({
      ...result,
      diagnostics: {
        ...result.diagnostics,
        last_outbound_notice: {
          code: "10005",
          reason: "telegram_formatting_fallback",
          at: "2026-11-10T09:00:00Z",
        },
      },
    });
    renderCard();
    expect(await screen.findByText("Formatting notice")).toBeInTheDocument();
    expect(screen.getByText(/attempted the same text without styling; see the delivery result for confirmation/)).toBeInTheDocument();
    expect(screen.getByText(/rate_limited at/)).toBeInTheDocument();
  });

  it("renders the diagnostics from /health with formatted times and errors", async () => {
    mockGet.mockResolvedValue(health());
    renderCard();
    expect(await screen.findByText("Needs attention")).toBeInTheDocument();
    expect(mockGet).toHaveBeenCalledWith("/channel-bots/bot-1/health");
    const inbound = new Date("2026-10-12T08:00:00Z").toLocaleString(getFormattingLocale());
    expect(screen.getByText(`${inbound} (webhook)`)).toBeInTheDocument();
    expect(screen.getByText("never")).toBeInTheDocument();
    expect(
      screen.getByText(
        `rate_limited at ${new Date("2026-10-12T09:00:00Z").toLocaleString(getFormattingLocale())}`,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/missing \(subscription_missing\), checked/),
    ).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
    expect(
      screen.getByText(/subscription is missing or incomplete/),
    ).toBeInTheDocument();
    expect(screen.queryByText("2026-10-12T08:00:00Z")).toBeNull();
  });

  it("offers exactly the recommended action and runs it", async () => {
    const user = userEvent.setup();
    mockGet.mockResolvedValue(health());
    mockPost.mockResolvedValue(
      health({
        recommended_action: "none",
        recommended_action_text: "Healthy.",
      }),
    );
    renderCard();
    const button = await screen.findByRole("button", { name: "Resubscribe" });
    expect(screen.queryByRole("button", { name: "Verify" })).toBeNull();
    await user.click(button);
    await waitFor(() =>
      expect(mockPost).toHaveBeenCalledWith(
        "/channel-bots/bot-1/resubscribe",
        {},
      ),
    );
  });

  it("hides the action from callers without write access", async () => {
    mockGet.mockResolvedValue(health({ can_write: false }));
    renderCard();
    expect(
      await screen.findByText(/missing or incomplete/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows advice without a button when the fix is elsewhere", async () => {
    mockGet.mockResolvedValue(
      health({
        recommended_action: "contact_admin",
        recommended_action_text: "An administrator must upgrade the X app.",
      }),
    );
    renderCard();
    expect(
      await screen.findByText("An administrator must upgrade the X app."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("reports a malformed health response instead of rendering it", async () => {
    mockGet.mockResolvedValue({ ...health(), can_write: undefined });
    renderCard();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Health is unavailable",
    );
  });
});


it("shows native hand-over evidence without presenting selection as healthy", async () => {
  mockGet.mockResolvedValue(health({platform:"discord", recommended_action:"contact_admin", recommended_action_text:"No native event confirmed.",
    discord_intake:{receiving_side:"native_gateway",state:"unconfirmed",code:"discord_native_intake_unconfirmed",block_reason:null,handed_over_at:"2026-11-07T00:00:00Z",last_inbound_at:null}}));
  renderCard();
  expect(await screen.findByText("Agent Event Gateway")).toBeInTheDocument();
  expect(screen.getByText("unconfirmed (discord_native_intake_unconfirmed)")).toBeInTheDocument();
  expect(screen.getByText("Last confirmed inbound")).toBeInTheDocument();
  expect(screen.queryByText("Healthy")).not.toBeInTheDocument();
  expect(screen.queryByText("Webhook", {exact:true})).not.toBeInTheDocument();
});

it("shows the relay block reason instead of implying a missing webhook", async () => {
  mockGet.mockResolvedValue(health({platform:"discord",recommended_action:"contact_admin",recommended_action_text:"Relay intake blocked.",
    discord_intake:{receiving_side:"nyxid_relay",state:"stopped",code:"discord_relay_blocked",block_reason:"release_unresolved",handed_over_at:null,last_inbound_at:null}}));
  renderCard();
  expect(await screen.findByText("NyxID relay")).toBeInTheDocument();
  expect(screen.getByText("release_unresolved")).toBeInTheDocument();
  expect(screen.queryByText("Healthy")).not.toBeInTheDocument();
});


it("keeps interactions-only Discord bots healthy without requesting relay verification", async () => {
  mockGet.mockResolvedValue(health({platform:"discord", recommended_action:"none", recommended_action_text:"Healthy.",
    discord_intake:{receiving_side:"interactions_only",state:"not_enabled",code:null,block_reason:null,handed_over_at:null,last_inbound_at:null}}));
  renderCard();
  expect(await screen.findByText("Discord interactions")).toBeInTheDocument();
  expect(screen.getByText("Healthy", {exact:true})).toBeInTheDocument();
  expect(screen.getByText("Interactions", {exact:true})).toBeInTheDocument();
  expect(screen.queryByRole("button", {name:"Verify"})).not.toBeInTheDocument();
  expect(screen.queryByText("Needs attention")).not.toBeInTheDocument();
});

it("keeps inbound counters in one collapsed disclosure while showing the latest outcome", async () => {
  const user = userEvent.setup();
  const base = health();
  mockGet.mockResolvedValue(health({
    recommended_action: "contact_admin",
    recommended_action_text: "An encrypted X Chat event arrived. NyxID cannot read or reply to its content. Use NyxID web.",
    diagnostics: {
      ...base.diagnostics,
      last_outbound_error: null,
      last_inbound_outcome: { code: "x_encrypted_chat_unsupported", at: "2026-11-11T01:00:00Z" },
      inbound_outcomes: {
        x_encrypted_chat_unsupported: { count: 3, at: "2026-11-11T01:00:00Z" },
        x_callback_pending: { count: 5, at: "2026-11-11T00:00:00Z" },
      },
    },
  }));
  renderCard();
  expect(await screen.findByText("Needs attention")).toBeInTheDocument();
  expect(screen.getByText("Last inbound outcome")).toBeVisible();
  expect(screen.getByText(/x_encrypted_chat_unsupported at/)).toBeVisible();
  const summary = screen.getByText("Inbound diagnostics");
  const disclosure = summary.closest("details");
  expect(screen.getAllByText("Inbound diagnostics")).toHaveLength(1);
  expect(disclosure).not.toHaveAttribute("open");
  expect(screen.getByText(/3 observed; last/)).not.toBeVisible();
  expect(screen.getByText("x_callback_pending")).not.toBeVisible();
  await user.click(summary);
  expect(disclosure).toHaveAttribute("open");
  expect(screen.getByText(/3 observed; last/)).toBeVisible();
  expect(screen.getByText(/5 observed; last/)).toBeVisible();
  await user.click(summary);
  expect(screen.getByText(/3 observed; last/)).not.toBeVisible();
  expect(screen.getByText("Last inbound outcome")).toBeVisible();
  expect(screen.getByText(/cannot read or reply/)).toBeInTheDocument();
  expect(screen.queryByText("Healthy", { exact: true })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Verify" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Resubscribe" })).not.toBeInTheDocument();
  expect(mockPost).not.toHaveBeenCalled();
});

it.each([
  { name: "fresh X bot with no traffic", code: null, label: "No problem detected" },
  { name: "external callback pending", code: "x_callback_pending", label: "No problem detected" },
  { name: "silent agent", code: "x_agent_silent", label: "No problem detected" },
  { name: "notification only", code: "x_notification_only", label: "No problem detected" },
  { name: "started turn", code: "x_turn_started", label: "No problem detected" },
  { name: "empty batch", code: "x_empty_batch", label: "No problem detected" },
  { name: "busy turn", code: "x_turn_busy", label: "No problem detected" },
  { name: "accepted reply", code: "x_reply_sent", label: "Reply accepted" },
])("uses the server's no-action state for $name", async ({ code, label }) => {
  const base = health();
  mockGet.mockResolvedValue(health({
    recommended_action: "none",
    recommended_action_text: "Encrypted X Chat remains unsupported.",
    diagnostics: {
      ...base.diagnostics,
      last_inbound_at: code ? "2026-11-11T01:00:00Z" : null,
      last_inbound_source: code ? "webhook" : null,
      last_webhook_at: code ? "2026-11-11T01:00:00Z" : null,
      last_outbound_at: code === "x_reply_sent" ? "2026-11-11T01:00:00Z" : null,
      last_outbound_error: null,
      subscription_state: { registered: true, checked_at: "2026-11-11T00:00:00Z", last_error: null },
      consecutive_errors: 0,
      failure_reason: null,
      ...(code ? { last_inbound_outcome: { code, at: "2026-11-11T01:00:00Z" } } : {}),
    },
  }));
  renderCard();
  expect(await screen.findByText(label, { exact: true })).toBeVisible();
  expect(screen.queryByText("Needs attention")).not.toBeInTheDocument();
  expect(screen.queryByText("Healthy", { exact: true })).not.toBeInTheDocument();
  expect(screen.queryByText("Inbound diagnostics")).not.toBeInTheDocument();
  if (code) {
    expect(screen.getByText("Last inbound outcome")).toBeVisible();
  } else {
    expect(screen.queryByText("Last inbound outcome")).not.toBeInTheDocument();
  }
  if (code !== "x_reply_sent") {
    expect(screen.queryByText("Reply accepted", { exact: true })).not.toBeInTheDocument();
  }
  expect(mockPost).not.toHaveBeenCalled();
});

it("shows Needs attention when callback failure has a server-recommended action", async () => {
  const base = health();
  mockGet.mockResolvedValue(health({
    recommended_action: "verify",
    recommended_action_text: "Check the linked agent and service health, then send a new message.",
    diagnostics: {
      ...base.diagnostics,
      last_outbound_error: null,
      subscription_state: { registered: true, checked_at: "2026-11-11T00:00:00Z", last_error: null },
      consecutive_errors: 0,
      failure_reason: null,
      last_inbound_outcome: { code: "x_callback_failed", at: "2026-11-11T01:00:00Z" },
    },
  }));
  renderCard();
  expect(await screen.findByText("Needs attention")).toBeVisible();
  expect(screen.getByRole("button", { name: "Verify" })).toBeVisible();
  expect(screen.getByText(/x_callback_failed at/)).toBeVisible();
  expect(screen.queryByText("No problem detected")).not.toBeInTheDocument();
  expect(mockPost).not.toHaveBeenCalled();
});


it.each([
  { language: "zh-CN", accepted: "回复已被接受", outbound: "上次出站尝试", outcome: "上次入站处理结果", diagnostics: "入站诊断", codeTime: "时间", count: "共记录 3 次；最近一次：" },
  { language: "zh-TW", accepted: "回覆已被接受", outbound: "上次傳出嘗試", outcome: "上次傳入處理結果", diagnostics: "傳入診斷", codeTime: "時間", count: "共記錄 3 次；最近一次：" },
])("localizes X diagnostic labels and counters in $language while preserving raw codes", async ({ language, accepted, outbound, outcome, diagnostics, codeTime, count }) => {
  await appI18n.changeLanguage(language);
  const user = userEvent.setup();
  const at = "2026-11-11T01:00:00Z";
  const base = health();
  mockGet.mockResolvedValue(health({
    recommended_action: "none",
    diagnostics: {
      ...base.diagnostics,
      last_outbound_error: null,
      last_inbound_outcome: { code: "x_reply_sent", at },
      inbound_outcomes: { x_reply_sent: { count: 3, at } },
    },
  }));
  renderCard();
  expect(await screen.findByText(accepted)).toBeVisible();
  expect(screen.getByText(outbound)).toBeVisible();
  expect(screen.getByText(outcome)).toBeVisible();
  const time = new Date(at).toLocaleString(getFormattingLocale());
  expect(screen.getByText(`x_reply_sent，${codeTime}：${time}`, { normalizer: (text) => text })).toBeVisible();
  expect(screen.getByText("x_reply_sent", { exact: true })).not.toBeVisible();
  const summary = screen.getByText(diagnostics);
  expect(summary.closest("details")).not.toHaveAttribute("open");
  await user.click(summary);
  expect(screen.getByText("x_reply_sent", { exact: true })).toBeVisible();
  expect(screen.getByText(`${count}${time}`, { normalizer: (text) => text })).toBeVisible();
  expect(screen.queryByText(/{{/)).not.toBeInTheDocument();
  expect(mockPost).not.toHaveBeenCalled();
});

it.each([
  { language: "zh-CN", action: "none", label: "未发现问题", code: "x_callback_pending" },
  { language: "zh-TW", action: "none", label: "未發現問題", code: "x_callback_pending" },
  { language: "zh-CN", action: "verify", label: "需要注意", code: "x_callback_failed" },
  { language: "zh-TW", action: "verify", label: "需要注意", code: "x_callback_failed" },
])("localizes $code in $language without changing the server attention decision", async ({ language, action, label, code }) => {
  const base = health();
  mockGet.mockResolvedValue(health({
    recommended_action: action,
    diagnostics: {
      ...base.diagnostics,
      last_outbound_error: null,
      last_inbound_outcome: { code, at: "2026-11-11T01:00:00Z" },
    },
  }));
  renderCard();
  expect(await screen.findByText(action === "none" ? "No problem detected" : "Needs attention")).toBeVisible();
  await act(async () => { await appI18n.changeLanguage(language); });
  expect(screen.getByText(label)).toBeVisible();
  expect(mockPost).not.toHaveBeenCalled();
});
