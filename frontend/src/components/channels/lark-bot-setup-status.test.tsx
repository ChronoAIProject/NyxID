import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { LarkBotSetupStatus, LarkBotSetupStatusView, LARK_VERIFY_SUCCESS } from "./lark-bot-setup-status";
import { nyxAgentChannelAgentSchema } from "@/schemas/assistant-nyxagent";
import { channelBotDiagnosticsSchema } from "@/schemas/channel-bot-health";
import { parseAssistantSearch } from "@/lib/assistant/search";
import type { NyxAgentChannelAgent } from "@/schemas/assistant-nyxagent";
import type { ChannelBotDetail } from "@/types/channels";

const state = vi.hoisted(() => ({ channels: [] as NyxAgentChannelAgent[] }));
vi.mock("@/hooks/use-channel-bots", () => ({ useChannelBotHealth: () => ({ refetch: vi.fn() }) }));
vi.mock("@/hooks/use-nyxbot-agents", () => ({ useNyxBotChannels: () => ({ data: state.channels, refetch: vi.fn() }) }));
vi.mock("@/hooks/use-org-specialist-channel", () => ({ useOrgSpecialistChannel: () => ({ settings: { data: { member_verified: false }, refetch: vi.fn() } }) }));

const connection = nyxAgentChannelAgentSchema.parse({
  id: "11111111-1111-4111-8111-111111111111", channel_bot_id: "bot", platform: "lark", bot_label: "RedQueen", transport: "direct", status: "active", owner_linked: false,
  agent_id: "22222222-2222-4222-8222-222222222222", created_at: "2026-11-05T00:00:00Z",
  org_specialist: { org_id: "org", guest_enabled: false, delegated_service_ids: [] },
});
const bot = { id: "bot", platform: "lark", conversations_count: 1, last_verification: { status: "verified" } } as ChannelBotDetail;
const empty = channelBotDiagnosticsSchema.parse({ last_inbound_at: null, last_inbound_source: null, last_outbound_at: null, last_outbound_error: null, last_webhook_at: null, last_webhook_error: null, subscription_state: null, consecutive_errors: 0, failure_reason: null });
afterEach(cleanup);
const value = (label: string) => within(screen.getByText(label).parentElement!).getByText((text, element) => element?.tagName === "SPAN" && text !== label);

it("keeps valid app credentials separate from unverified membership and links the exact connection", () => {
  render(<LarkBotSetupStatusView bot={bot} connection={connection} connectionsLoaded diagnostics={empty} memberVerified={false} onRefresh={vi.fn()} />);
  expect(value("App credentials")).toHaveTextContent("App credentials verified only");
  expect(value("Your member identity")).toHaveTextContent("Not verified");
  expect(value("Authenticated inbound")).toHaveTextContent("No real message observed yet");
  expect(value("Outbound delivery")).toHaveTextContent("No successful outbound observed yet");
  const link = screen.getByRole("link", { name: "Continue setup / Verify my chat account" });
  const url = new URL(link.getAttribute("href")!, "https://nyx.example");
  expect(parseAssistantSearch(Object.fromEntries(url.searchParams))).toEqual({ agent: connection.agent_id, details: "channels", connection: connection.id });
  expect(screen.getByText(/Each organization member, including admins/)).toBeVisible();
  expect(LARK_VERIFY_SUCCESS).toContain("App credentials verified");
  expect(LARK_VERIFY_SUCCESS).toContain("does not verify");
});

it("a challenge alone proves neither a real message nor member identity", () => {
  render(<LarkBotSetupStatusView bot={bot} connection={connection} connectionsLoaded diagnostics={{ ...empty, last_callback_challenge_at: "2026-11-05T00:00:00Z" }} onRefresh={vi.fn()} />);
  expect(value("Callback challenge")).toHaveTextContent("Last observed");
  expect(value("Authenticated inbound")).toHaveTextContent("No real message observed");
  expect(value("Your member identity")).toHaveTextContent("Verification status unavailable");
  expect(screen.getByText(/Not yet observed does not mean misconfigured/)).toBeVisible();
});

it("received-with-no-route is different from no inbound; legacy observations stay unknown", () => {
  render(<LarkBotSetupStatusView bot={bot} connectionsLoaded diagnostics={{ ...empty, last_inbound_at: "2026-11-05T00:00:00Z", last_inbound_source: "webhook", last_route_result: "no_route" }} onRefresh={vi.fn()} />);
  expect(value("Authenticated inbound")).toHaveTextContent("via webhook");
  expect(value("Route / agent connection")).toHaveTextContent("Message received, but no eligible route was found");
  expect(value("Callback challenge")).toHaveTextContent("Not yet observed");
});

it("shows an actual failed outbound separately from missing evidence", () => {
  render(<LarkBotSetupStatusView bot={bot} connection={connection} connectionsLoaded memberVerified diagnostics={{ ...empty, last_outbound_error: { code: "10005", reason: "provider_unavailable", at: "2026-11-05T00:00:00Z" } }} onRefresh={vi.fn()} />);
  expect(value("Outbound delivery")).toHaveTextContent("Last send failed (provider_unavailable)");
  expect(value("Your member identity")).toHaveTextContent("Your chat account is verified");
  expect(value("Callback challenge")).toHaveTextContent("Not yet observed");
});


it("continues the live org connection when a retired binding from another admin is also listed", () => {
  state.channels = [{ ...connection, id: "33333333-3333-4333-8333-333333333333", status: "failed" }, connection];
  render(<LarkBotSetupStatus bot={bot} />);
  expect(screen.getByRole("link", { name: "Continue setup / Verify my chat account" }).getAttribute("href")).toContain(`connection=${connection.id}`);
});
