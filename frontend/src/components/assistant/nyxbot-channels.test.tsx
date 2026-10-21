import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { assistantAgentSchema, nyxAgentChannelAgentSchema, type AssistantAgent } from "@/schemas/assistant-nyxagent";
import { ChannelBotsManager } from "./nyxbot-channels";

const data = vi.hoisted(() => ({
  channels: [] as unknown[], bots: [] as unknown[],
  connect: vi.fn(), link: vi.fn(), pending: false,
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  nyxBotOf: (agents: AssistantAgent[]) => agents.find((agent) => agent.kind === "nyxbot"),
  useNyxBotChannels: () => ({ data: data.channels }),
  useConnectNyxBotChannel: () => ({ isPending: data.pending, mutateAsync: data.connect }),
  useDisconnectNyxBotChannel: () => ({ isPending: false }),
  useLinkNyxBotChannel: () => ({ isPending: false, mutateAsync: data.link }),
}));
vi.mock("@/hooks/use-channel-bots", () => ({
  useChannelBots: () => ({ data: data.bots, isPending: false }),
}));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: [] }) }));
vi.mock("./nyxbot-channel-chats", () => ({ ChannelChats: () => null }));
beforeEach(() => {
  data.channels = []; data.bots = []; data.pending = false;
  data.connect.mockResolvedValue({ channel_agent: { bot_label: "Receipts" }, link: {
    code: "LINK123", instructions: "Send this code to your bot.", expires_at: "2026-10-22T00:00:00Z",
  } });
  data.link.mockResolvedValue(undefined);
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });

function mount(status?: string, reason?: string) {
  data.channels = [nyxAgentChannelAgentSchema.parse({
    id: "channel", channel_bot_id: "bot", platform: "lark", bot_label: "Receipts",
    transport: "gateway", status: "active", owner_linked: true,
    delivery_status: status, delivery_reason: reason, created_at: "2026-10-01T00:00:00Z",
  })];
  render(<ChannelBotsManager agents={[]} />);
}

it.each([
  ["failing", "Messages are not reaching the agent", "the gateway never passed it on"],
  ["interrupted", "Attachment problem", "a photo was interrupted before processing"],
  ["media_failed", "Attachment problem", "a photo or file exceeded the size limit"],
])("shows %s with its server reason and existing error style", (status, title, reason) => {
  mount(status, reason);
  const line = screen.getByText(new RegExp(title));
  expect(line).toHaveTextContent(reason);
  expect(line).toHaveClass("text-11", "text-destructive");
});

it.each([undefined, "ok"])("keeps legacy and healthy status %s quiet", (status) => {
  mount(status);
  expect(screen.queryByText(/Messages are not reaching|Attachment problem/)).not.toBeInTheDocument();
});


const at = "2026-10-01T00:00:00Z";
function agent(id: string, status = "idle") {
  return assistantAgentSchema.parse({ id, name: id, kind: "specialist", status, created_at: at, last_active_at: at });
}
function connection(status: string, error?: string) {
  return nyxAgentChannelAgentSchema.parse({ id: "channel", channel_bot_id: "bot", platform: "lark",
    bot_label: "Receipts", agent_id: "first", transport: "gateway", status, owner_linked: true,
    created_at: at, last_error: error });
}

it("reconnects a failed row to its selected agent and shows returned owner instructions", async () => {
  data.channels = [connection("failed")];
  data.bots = [{ id: "bot", label: "Receipts", platform: "lark" }];
  render(<ChannelBotsManager agents={[agent("first"), agent("second")]} />);
  expect(screen.queryByRole("button", { name: /Link account|Link another account|^Connect / })).not.toBeInTheDocument();
  await userEvent.setup().click(screen.getByRole("button", { name: "Reconnect" }));
  expect(data.connect).toHaveBeenCalledWith({ botId: "bot", agentId: "first" });
  expect(data.link).not.toHaveBeenCalled();
  expect(await screen.findByRole("region", { name: "Owner link" })).toHaveTextContent("Send this code to your bot.");
  expect(screen.getByRole("button", { name: "Disconnect" })).toBeEnabled();
});

it.each(["failed", "pending", "active"])("changing the agent on a %s row uses the correct mutation", async (status) => {
  data.channels = [connection(status)];
  render(<ChannelBotsManager agents={[agent("first"), agent("second")]} />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name: "Agent for Receipts" }));
  await user.click(screen.getByRole("option", { name: "second" }));
  if (status === "failed") {
    expect(data.connect).toHaveBeenCalledWith({ botId: "bot", agentId: "second" });
    expect(data.link).not.toHaveBeenCalled();
    expect(await screen.findByRole("region", { name: "Owner link" })).toBeVisible();
  } else {
    expect(data.link).toHaveBeenCalledWith({ channelAgentId: "channel", agentId: "second" });
    expect(data.connect).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Link another account" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Reconnect" })).not.toBeInTheDocument();
  }
});

it.each(["missing", "destroyed", "pending"])("disables reconnect when target is %s", (condition) => {
  data.channels = [connection("failed")];
  data.pending = condition === "pending";
  render(<ChannelBotsManager agents={condition === "missing" ? [] : [agent("first", condition === "destroyed" ? "destroyed" : "idle")]} />);
  expect(screen.getByRole("button", { name: "Reconnect" })).toBeDisabled();
});

it.each([
  ["gateway_not_authenticated", "The channel gateway could not authenticate NyxBot. Try reconnecting. If this continues, contact an administrator."],
  ["gateway_permission_required", "The channel gateway has not authorized NyxBot to manage this connection. An administrator needs to check the gateway configuration."],
  ["gateway_unavailable", "The channel gateway is temporarily unavailable. Try again shortly."],
  ["setup_interrupted", "This connection changed while it was being set up. Reconnect to try again."],
  ["gateway_provider_not_registered", "The Agent Event Gateway has not registered NyxBot as a provider yet (operator step). Telegram bots cannot use NyxBot until it is."],
  ["gateway_rejected_request", "The channel gateway rejected this connection request. Contact an administrator."],
  ["gateway_conflict", "The channel gateway reported a conflicting connection. Contact an administrator."],
  ["gateway_provider_rejected", "The channel gateway could not complete setup with NyxBot. Contact an administrator."],
  ["gateway_invalid_response", "The channel gateway returned an invalid connection response. Contact an administrator."],
  ["issuer_unsupported", "NyxBot cannot connect with this server's issuer configuration. Contact an administrator."],
  ["creator_unavailable", "NyxBot could not prepare its channel gateway credential. Contact an administrator."],
  ["storage_unavailable", "NyxBot could not save this connection. Try again shortly."],
  ["route_key_create_failed", "NyxBot could not prepare this bot's connection credential. Try again shortly."],
  ["route_key_update_failed", "NyxBot could not configure this bot's connection credential. Try again shortly."],
  ["route_create_failed", "NyxBot could not create this bot's message route. Try again shortly."],
  ["org_access_lost", "You no longer administer the organization that owns this bot, so NyxBot stopped answering through it."],
  ["future_code", "This connection failed."],
  ["__proto__", "This connection failed."],
])("explains last error %s and keeps the code as secondary text", (code, explanation) => {
  data.channels = [connection("failed", code)];
  render(<ChannelBotsManager agents={[]} />);
  expect(screen.getByText(`Last error: ${explanation}`)).toBeVisible();
  expect(screen.getByText(code)).toHaveClass("text-text-tertiary");
});
