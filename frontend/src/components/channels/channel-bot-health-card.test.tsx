import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
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

beforeEach(() => vi.clearAllMocks());

describe("ChannelBotHealthCard", () => {
  it("renders the diagnostics from /health with formatted times and errors", async () => {
    mockGet.mockResolvedValue(health());
    renderCard();
    expect(await screen.findByText("Needs attention")).toBeInTheDocument();
    expect(mockGet).toHaveBeenCalledWith("/channel-bots/bot-1/health");
    const inbound = new Date("2026-10-12T08:00:00Z").toLocaleString();
    expect(screen.getByText(`${inbound} (webhook)`)).toBeInTheDocument();
    expect(screen.getByText("never")).toBeInTheDocument();
    expect(
      screen.getByText(
        `rate_limited at ${new Date("2026-10-12T09:00:00Z").toLocaleString()}`,
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
