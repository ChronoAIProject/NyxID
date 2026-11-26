import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render as renderView, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { AddRouteDialog, ChannelBotDetailPage } from "./channel-bot-detail";

function render(element: React.ReactElement) {
  const client = new QueryClient();
  client.setQueryData(["channel-guest-setup", "bot", false], { available: false, enabled: false, bot_revision: 0 });
  return renderView(element, {
    wrapper: ({ children }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  });
}

const { refetch, apiKeys, queryState, create, verify, botFixture } = vi.hoisted(() => ({
  refetch: vi.fn(),
  apiKeys: vi.fn(),
  queryState: { error: null as Error | null },
  create: vi.fn(),
  verify: vi.fn(),
  botFixture: { platform: "lark", owner: "org-1", status: "active" },
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useApiKeys: (scope: unknown) => {
    apiKeys(scope);
    return { data: [], isLoading: false, error: queryState.error, refetch };
  },
}));
vi.mock("@/hooks/use-channel-conversations", () => ({
  useChannelConversations: () => ({ data: [] }),
  useDeleteChannelConversation: () => ({ mutateAsync: vi.fn() }),
  useCreateChannelConversation: () => ({ mutate: create, isPending: false }),
}));
vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
  queryState.error = null;
  Object.assign(botFixture, { platform: "lark", owner: "org-1", status: "active" });
  verify.mockReset().mockReturnValue({ mutateAsync: vi.fn() });
});

it("loads the bot owner's keys and disables submission with no eligible keys", () => {
  render(
    <AddRouteDialog
      open
      onOpenChange={vi.fn()}
      botId="bot"
      ownerOrgId="org-1"
      ownerLabel="ChronoAI"
      isTelegramManager={false}
    />,
  );
  expect(apiKeys).toHaveBeenCalledWith({ orgId: "org-1" });
  expect(screen.getByRole("button", { name: "Add Route" })).toBeDisabled();
  expect(screen.getByText(/No agent keys exist/)).toBeInTheDocument();
  expect(create).not.toHaveBeenCalled();
});

it("has a working retry control when key loading fails", () => {
  queryState.error = new Error("offline");
  render(
    <AddRouteDialog
      open
      onOpenChange={vi.fn()}
      botId="bot"
      ownerOrgId="org-1"
      ownerLabel="ChronoAI"
      isTelegramManager={false}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(refetch).toHaveBeenCalledOnce();
  expect(screen.getByRole("button", { name: "Add Route" })).toBeDisabled();
});

vi.mock("@/hooks/use-runtime-config", () => ({
  useRuntimeConfig: () => ({ data: { api_base_url: "https://nyx.example" } }),
}));
vi.mock("@/hooks/use-channel-platforms", () => ({
  useChannelPlatformViews: () => ({
    getPlatform: (id: string) => ({ label: id === "discord" ? "Discord" : "Slack", webhookDocs: null, activities: [], fields: [] }),
  }),
}));

it("explains Gateway activation and optional Discord interactions without generic webhook directions", async () => {
  const { WebhookSetupChecklist } = await import("./channel-bot-detail");
  render(<WebhookSetupChecklist bot={{ id: "bot", platform: "discord", status: "pending_webhook" } as import("@/types/channels").ChannelBotDetail} />);
  expect(screen.getByText("Finish Discord setup")).toBeInTheDocument();
  expect(screen.getByText(/Ordinary chat arrives through the Gateway connection/)).toHaveTextContent("first authenticated message activates the bot");
  expect(screen.getByText("Interactions Endpoint URL (optional)")).toBeInTheDocument();
  expect(screen.getByText(/For slash commands\/components, paste this into Discord Developer Portal/)).toBeInTheDocument();
  expect(screen.queryByText(/Event Subscriptions \/ webhook settings/)).not.toBeInTheDocument();
  expect(screen.queryByText("Finish webhook setup")).not.toBeInTheDocument();
});

it("keeps the existing webhook setup instructions for other platforms", async () => {
  const { WebhookSetupChecklist } = await import("./channel-bot-detail");
  render(<WebhookSetupChecklist bot={{ id: "bot", platform: "slack", status: "pending_webhook" } as import("@/types/channels").ChannelBotDetail} />);
  expect(screen.getByText("Finish webhook setup")).toBeInTheDocument();
  expect(screen.getByText(/Slack's Event Subscriptions \/ webhook settings/)).toBeInTheDocument();
});

it("distinguishes Lark's callback challenge from activation and uninspected scopes", async () => {
  const { WebhookSetupChecklist } = await import("./channel-bot-detail");
  render(<WebhookSetupChecklist bot={{ id: "bot", platform: "lark", status: "pending_webhook", permission_setup_url: "https://example.com/permissions" } as import("@/types/channels").ChannelBotDetail} />);
  expect(screen.getByText("Finish Lark / Feishu setup")).toBeVisible();
  expect(screen.getByText(/The URL challenge checks callback reachability only/)).toHaveTextContent("does not activate the bot or verify a member");
  expect(screen.getByText(/Not inspected by NyxID/)).toBeVisible();
});


it("does not call legacy app credentials missing after a successful check", async () => {
  const { WebhookSetupChecklist } = await import("./channel-bot-detail");
  render(<WebhookSetupChecklist bot={{ id: "bot", platform: "lark", status: "pending_webhook", app_secret_configured: false, last_verification: { status: "verified" } } as import("@/types/channels").ChannelBotDetail} />);
  expect(screen.getByText("App credentials verified. Callback and member verification are separate.")).toBeVisible();
  expect(screen.queryByText(/If credentials are missing/)).not.toBeInTheDocument();
});


vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ botId: "org-bot" }),
  useNavigate: () => vi.fn(),
  Link: ({ children }: { children: ReactNode }) => <>{children}</>,
}));
vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({ data: [{ id: "org-1", display_name: "Team", your_role: "member" }] }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: { user: { id: string } }) => unknown) => select({ user: { id: "member" } }),
}));
vi.mock("@/hooks/use-channel-bots", () => ({
  useChannelBot: () => ({ data: {
    id: "org-bot", user_id: botFixture.owner, platform: botFixture.platform, label: "Team helper",
    created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z",
    status: botFixture.status, platform_bot_username: "team_helper", conversations_count: 7,
    setup_instructions: ["Existing admin setup details"],
  }, isLoading: false }),
  useDeleteChannelBot: () => ({ mutateAsync: vi.fn() }),
  useVerifyChannelBot: () => verify(),
  useUpdateChannelBot: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-channel-activities", () => ({
  useChannelActivities: () => ({ data: undefined }),
}));
vi.mock("@/components/channels/channel-bot-health-card", () => ({
  ChannelBotHealthCard: () => <p>Bot health</p>,
}));

it("uses membership for read-only settings with main's detail response and no can_write field", () => {
  render(<ChannelBotDetailPage />);
  expect(screen.getByText("An organization admin can change these settings.")).toBeVisible();
  expect(screen.getByText("Bot health")).toBeVisible();
  expect(apiKeys).toHaveBeenCalledWith({ orgId: "org-1", enabled: false });
  expect(screen.queryByRole("button", { name: /Delete Bot|Add Route|Verify/i })).not.toBeInTheDocument();
});

vi.mock("@/components/shared/ownership-transfer-card", () => ({ OwnershipTransferCard: () => null }));

it.each([
  ["pending_webhook", "credentials", "Credentials verified. Bot status: Pending Webhook."],
  ["active", "credentials", "Credentials verified. Bot status: Active."],
  ["active", undefined, "Check complete. See bot details for the current status."],
] as const)("reports the Discord verification result for %s with scope %s", async (status, scope, message) => {
  Object.assign(botFixture, { platform: "discord", owner: "member", status: "pending_webhook" });
  const result = { id: "org-bot", verification_scope: scope, status, webhook_registered: !scope };
  const mutateAsync = vi.fn().mockResolvedValue(result);
  verify.mockReturnValue({ mutateAsync, isSuccess: true, variables: result.id, data: result });
  const success = vi.spyOn(toast, "success");
  render(<ChannelBotDetailPage />);
  fireEvent.click(screen.getByRole("button", { name: "Verify Bot" }));
  await waitFor(() => expect(success).toHaveBeenCalledWith(message));
  expect(screen.getByText(message)).toBeVisible();
});

vi.mock("@/components/channels/meta-customer-setup", () => ({ MetaCustomerSetup: () => null }));

it.each(["messenger", "instagram"])("guides manual %s credential repair according to the delivery hold", (platform) => {
  Object.assign(botFixture, { platform, owner: "member", status: "suspended" });
  const page = render(<ChannelBotDetailPage />);
  expect(screen.getByText(/nyxid channel-bot delivery org-bot --resume/)).toBeVisible();
  botFixture.status = "pending_webhook";
  page.rerender(<ChannelBotDetailPage />);
  expect(screen.queryByText(/nyxid channel-bot delivery org-bot --resume/)).toBeNull();
  expect(screen.getByText(/Run Verify Bot to check the saved credentials/)).toBeVisible();
});
