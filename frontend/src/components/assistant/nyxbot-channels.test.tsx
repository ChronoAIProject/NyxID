import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { nyxAgentChannelAgentSchema } from "@/schemas/assistant-nyxagent";
import { ChannelBotsManager } from "./nyxbot-channels";

const data = vi.hoisted(() => ({ channels: [] as unknown[] }));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  nyxBotOf: () => undefined,
  useNyxBotChannels: () => ({ data: data.channels }),
  useConnectNyxBotChannel: () => ({ isPending: false }),
  useDisconnectNyxBotChannel: () => ({ isPending: false }),
  useLinkNyxBotChannel: () => ({ isPending: false }),
}));
vi.mock("@/hooks/use-channel-bots", () => ({
  useChannelBots: () => ({ data: [], isPending: false }),
}));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: [] }) }));
vi.mock("./nyxbot-channel-chats", () => ({ ChannelChats: () => null }));
afterEach(cleanup);

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
