import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { AddRouteDialog } from "./channel-bot-detail";

const { refetch, apiKeys, queryState, create } = vi.hoisted(() => ({
  refetch: vi.fn(),
  apiKeys: vi.fn(),
  queryState: { error: null as Error | null },
  create: vi.fn(),
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useApiKeys: (scope: unknown) => {
    apiKeys(scope);
    return { data: [], isLoading: false, error: queryState.error, refetch };
  },
}));
vi.mock("@/hooks/use-channel-conversations", () => ({
  useCreateChannelConversation: () => ({ mutate: create, isPending: false }),
}));
vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
  queryState.error = null;
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
    getPlatform: (id: string) => ({ label: id === "discord" ? "Discord" : "Slack", webhookDocs: null }),
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
