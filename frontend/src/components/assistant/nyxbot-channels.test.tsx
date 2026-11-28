import { formatDateTime } from "@/lib/utils";
import { appI18n } from "@/i18n/instance";
import zh from "@/i18n/locales/app/zh-CN";
import { act, cleanup, render as renderView, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { assistantAgentSchema, nyxAgentChannelAgentSchema, type AssistantAgent } from "@/schemas/assistant-nyxagent";
import { ChannelBotsManager } from "./nyxbot-channels";
import { ApiError } from "@/lib/api-client";

vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: { get: vi.fn().mockResolvedValue({ available: false, enabled: false, bot_revision: 0 }) },
}));

function render(element: React.ReactElement) {
  const client = new QueryClient();
  return renderView(element, {
    wrapper: ({ children }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  });
}

const data = vi.hoisted(() => ({
  channels: [] as unknown[],
  bots: [] as unknown[],
  orgs: [] as unknown[],
  orgChannels: false,
  connect: vi.fn(),
  preview: vi.fn(),
  link: vi.fn(),
  retryGateway: vi.fn(),
  pending: false,
  refetchBots: vi.fn(),
  setupProps: undefined as Record<string, unknown> | undefined,
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  nyxBotOf: (agents: AssistantAgent[]) => agents.find((agent) => agent.kind === "nyxbot"),
  useNyxBotChannels: () => ({ data: data.channels }),
  usePreviewOrgChannel: () => ({ isPending: false, mutateAsync: data.preview }),
  useConnectNyxBotChannel: () => ({ isPending: data.pending, mutateAsync: data.connect }),
  useDisconnectNyxBotChannel: () => ({ isPending: false }),
  useLinkNyxBotChannel: () => ({ isPending: false, mutateAsync: data.link }),
  useRetryNyxBotChannelGateway: () => ({ isPending: false, mutateAsync: data.retryGateway }),
}));
vi.mock("@/hooks/use-channel-bots", () => ({
  useChannelBots: () => ({ data: data.bots, isPending: false, refetch: data.refetchBots }),
}));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: data.orgs }) }));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => data.orgChannels,
}));
vi.mock("./bot-audience-settings", () => ({
  BotAudienceSettings: ({ channel, canManage, busy, onReconnect }: {
    channel: { status: string }; canManage: boolean; busy: boolean; onReconnect: () => void;
  }) => <section aria-label="Who can talk to this bot">
    {channel.status !== "active" && canManage ? <button disabled={busy} onClick={onReconnect}>
      {channel.status === "pending" ? "Finish connecting" : "Reconnect"}
    </button> : null}
  </section>,
}));
vi.mock("./org-specialist-channel-settings", () => ({
  OrgSpecialistChannelSettings: () => <section data-member-verification="" tabIndex={-1} aria-label="Verify my chat account">Member verification</section>,
}));
vi.mock("./nyxbot-channel-chats", () => ({
  ChannelChats: ({ agents, readOnly }: { agents: readonly AssistantAgent[]; readOnly?: boolean }) => (
    <div data-testid="channel-chat-agents" data-read-only={readOnly}>
      {agents.map((agent) => <span key={agent.id}>{agent.name}</span>)}
    </div>
  ),
}));
vi.mock("@/components/channels/channel-bot-setup", () => ({
  ChannelBotSetup: (props: { onOpenChange: (open: boolean) => void; onContinue: () => Promise<void>; continueLabel: string }) => {
    data.setupProps = props;
    return <section aria-label="Registration"><button onClick={() => props.onOpenChange(false)}>Cancel registration</button><button onClick={() => void props.onContinue()}>{props.continueLabel}</button></section>;
  },
}));
beforeEach(() => {
  data.channels = [];
  data.bots = [];
  data.orgs = [];
  data.orgChannels = false;
  data.pending = false;
  data.connect.mockResolvedValue({
    channel_agent: { bot_label: "Receipts" },
    link: {
      code: "LINK123",
      instructions: "Send this code to your bot.",
      expires_at: "2026-10-22T00:00:00Z",
    },
  });
  data.link.mockResolvedValue(undefined);
  data.preview.mockResolvedValue({
    bot_id:"owned",agent_id:"org-specialist",org_id:"team",guest_enabled:true,private_chats:"everyone",guest_policy_mode:"grants",
    description:"Guests can talk privately and in groups. The organization pays for guest messages and tool use.",
    effective_guest_services:[{id:"calendar",label:"Calendar",access:"use",limited_operations:true}],
    audience_readiness:{guest_readiness:{status:"ready"},configuration:{instructions_present:true,persona_present:false,memory_notes:0,skills:0,service_grants:1,account_read:true},note:"Configuration facts"},health:null,
  });
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
  expect(line).toHaveClass("text-13", "text-destructive");
});

it.each([undefined, "ok"])("keeps legacy and healthy status %s quiet", (status) => {
  mount(status);
  expect(screen.queryByText(/Messages are not reaching|Attachment problem/)).not.toBeInTheDocument();
});

it.each([
  ["gateway", "native_gateway", "unconfirmed", "Agent Event Gateway", "unconfirmed", null],
  ["direct", "nyxid_relay", "failing", "NyxID relay", "stopped", "release_unresolved"],
])("shows %s Discord intake evidence without claiming delivery", (transport, side, status, label, state, block) => {
  data.channels = [nyxAgentChannelAgentSchema.parse({
    id: "channel", channel_bot_id: "bot", platform: "discord", bot_label: "Receipts",
    transport, status: "active", owner_linked: true, delivery_status: status,
    delivery_reason: "Intake needs confirmation.", created_at: "2026-10-01T00:00:00Z",
    discord_intake: { receiving_side: side, state, code: null, block_reason: block, handed_over_at: null, last_inbound_at: null },
  })];
  render(<ChannelBotsManager agents={[]} />);
  expect(screen.getByText(`Receiving side: ${label}. Intake: ${state}.`)).toBeVisible();
  expect(screen.getByText("Last confirmed inbound: not observed.")).toBeVisible();
  expect(screen.getByText(/Intake needs confirmation/)).toBeVisible();
  if (block) expect(screen.getByText(`Intake blocked by: ${block}.`)).toBeVisible();
  expect(screen.queryByText(/Connected through the Agent Event Gateway/)).not.toBeInTheDocument();
});

const at = "2026-10-01T00:00:00Z";
function agent(id: string, status = "idle", owner_kind?: "person" | "org") {
  return assistantAgentSchema.parse({ id, name: id, kind: "specialist", owner_kind, status, created_at: at, last_active_at: at });
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
  expect(
    screen.queryByRole("button", {
      name: /^Link account$|^Link another account$|^Connect Receipts/,
    }),
  ).not.toBeInTheDocument();
  await userEvent.setup().click(screen.getByRole("button", { name: "Reconnect" }));
  expect(data.connect).toHaveBeenCalledWith({ botId: "bot", agentId: "first" });
  expect(data.link).not.toHaveBeenCalled();
  expect(await screen.findByRole("region", { name: "Owner link" })).toHaveTextContent("Send this code to your bot.");
  expect(screen.getByRole("button", { name: "Disconnect" })).toBeEnabled();
});

it.each(
  [
    ["gateway_not_authenticated", "could not authenticate NyxBot"],
    ["gateway_permission_required", "has not authorized NyxBot"],
    [
      "gateway_configuration_required",
      "check the active cmaeg service and its URL",
    ],
    ["gateway_unavailable", "temporarily unavailable"],
  ].flatMap(([code, guidance]) => [
    ["connect", code, guidance],
    ["reconnect", code, guidance],
  ]),
)("explains a rejected %s mutation with %s", async (action, code, guidance) => {
  const target = agent("first");
  data.channels = action === "reconnect" ? [connection("failed")] : [];
  data.bots = [{ id: "bot", label: "Receipts", platform: "lark" }];
  data.connect.mockRejectedValueOnce(new ApiError(400, {
    error: "validation_error", error_code: 1008, message: `Validation error: ${code}`,
  }));
  render(<ChannelBotsManager agents={[target]} agent={target} />);
  if (action === "connect")
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Connect another bot" }));
  await userEvent.setup().click(screen.getByRole("button", {
    name: action === "reconnect" ? "Reconnect" : "Connect Receipts to first",
  }));
  expect(await screen.findByRole("alert")).toHaveTextContent(guidance!);
  expect(screen.getByRole("alert")).not.toHaveTextContent(code!);
  expect(data.connect).toHaveBeenCalledWith({ botId: "bot", agentId: "first" });
  expect(screen.queryByRole("region", { name: "Owner link" })).not.toBeInTheDocument();
});

it.each([
  ["Choose an active bot before connecting.", "Choose an active bot before connecting."],
  ["Validation error: Choose an active bot before connecting.", "Validation error: Choose an active bot before connecting."],
  ["Validation error: gateway_future_code", "This connection failed."],
  ["Validation error: gateway_unavailable is only an example.", "Validation error: gateway_unavailable is only an example."],
])("preserves ordinary mutation guidance and safely handles %s", async (message, expected) => {
  const target = agent("first");
  data.channels = [connection("failed")];
  data.connect.mockRejectedValueOnce(new ApiError(400, {
    error: "validation_error", error_code: 1008, message,
  }));
  render(<ChannelBotsManager agents={[target]} />);
  await userEvent.setup().click(screen.getByRole("button", { name: "Reconnect" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(expected);
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

it("does not offer organization specialists as new channel targets", async () => {
  data.bots = [{ id: "new-bot", label: "New bot", platform: "lark" }];
  render(<ChannelBotsManager agents={[agent("personal"), agent("org", "idle", "org")]} />);
  await userEvent
    .setup()
    .click(screen.getByRole("button", { name: "Connect another bot" }));
  await userEvent.setup().click(screen.getByRole("combobox", { name: "Agent for New bot" }));
  expect(screen.getByRole("option", { name: "personal" })).toBeVisible();
  expect(screen.queryByRole("option", { name: "org" })).not.toBeInTheDocument();
});

it("does not offer organization specialists as per-chat targets", () => {
  data.channels = [connection("active")];
  render(<ChannelBotsManager agents={[agent("first"), agent("org", "idle", "org")]} />);
  expect(screen.getByTestId("channel-chat-agents")).toHaveTextContent("first");
  expect(screen.getByTestId("channel-chat-agents")).not.toHaveTextContent("org");
});

it("does not offer organization specialists when relinking connected bots", async () => {
  data.channels = [connection("active")];
  render(<ChannelBotsManager agents={[agent("first"), agent("org", "idle", "org")]} />);
  await userEvent.setup().click(screen.getByRole("combobox", { name: "Agent for Receipts" }));
  expect(screen.getByRole("option", { name: "first" })).toBeVisible();
  expect(screen.queryByRole("option", { name: "org" })).not.toBeInTheDocument();
});

it.each([
  [
    "gateway_not_authenticated",
    "The channel gateway could not authenticate NyxBot. Try reconnecting. If this continues, contact an administrator.",
  ],
  [
    "gateway_permission_required",
    "The channel gateway has not authorized NyxBot to manage this connection. An administrator needs to check the gateway configuration.",
  ],
  [
    "gateway_configuration_required",
    "The channel gateway needs administrator configuration. Ask an administrator to check the active cmaeg service and its URL.",
  ],
  [
    "gateway_unavailable",
    "The channel gateway is temporarily unavailable. Try again shortly.",
  ],
  [
    "setup_interrupted",
    "This connection changed while it was being set up. Reconnect to try again.",
  ],
  [
    "gateway_provider_not_registered",
    "The gateway has not registered NyxBot as a provider yet (operator step). Ask an administrator to register the nyxbot provider, then reconnect after the 60-second retry delay.",
  ],
  [
    "gateway_rejected_request",
    "The gateway rejected the connection request (HTTP 400). Ask an administrator to check the gateway request contract before reconnecting. Bot verification and owner pairing do not fix this rejection.",
  ],
  [
    "gateway_conflict",
    "The channel gateway reported a conflicting connection. Contact an administrator.",
  ],
  [
    "gateway_provider_rejected",
    "The channel gateway could not complete setup with NyxBot. Contact an administrator.",
  ],
  [
    "gateway_invalid_response",
    "The channel gateway returned an invalid connection response. Contact an administrator.",
  ],
  [
    "issuer_unsupported",
    "NyxBot cannot connect with this server's issuer configuration. Contact an administrator.",
  ],
  [
    "creator_unavailable",
    "NyxBot could not prepare its channel gateway credential. Contact an administrator.",
  ],
  [
    "storage_unavailable",
    "NyxBot could not save this connection. Try again shortly.",
  ],
  [
    "route_key_create_failed",
    "NyxBot could not prepare this bot's connection credential. Try again shortly.",
  ],
  [
    "route_key_update_failed",
    "NyxBot could not configure this bot's connection credential. Try again shortly.",
  ],
  [
    "route_create_failed",
    "NyxBot could not create this bot's message route. Try again shortly.",
  ],
  [
    "org_access_lost",
    "You no longer administer the organization that owns this bot, so NyxBot stopped answering through it.",
  ],
  [
    "org_access_restoration_pending",
    "This organization channel is being restored automatically; it normally returns in under a minute. Members must verify their chat account again.",
  ],
  [
    "org_access_restoration_waiting_readiness",
    "This organization channel is waiting for required safety checks to become available. NyxID will retry automatically; no administrator rights were lost.",
  ],
  [
    "org_access_restoration_retrying_gateway",
    "The channel connection is retrying restoration. The retry schedule can take hours; no administrator rights were lost.",
  ],
  [
    "org_access_restored",
    "This organization channel was restored. Members must verify their chat account again. Guest access is preserved as before.",
  ],
  ["future_code", "This connection failed."],
  ["__proto__", "This connection failed."],
])(
  "explains last error %s and keeps its code as diagnostic metadata",
  (code, explanation) => {
    data.channels = [connection("failed", code)];
    render(<ChannelBotsManager agents={[]} />);
    expect(screen.getByText(`Last error: ${explanation}`)).toBeVisible();
    expect(screen.queryByText(code)).not.toBeInTheDocument();
    expect(screen.getByText(`Last error: ${explanation}`)).toHaveAttribute(
      "data-channel-error",
      code,
    );
  },
);

it("offers an enabled organization specialist only to its admin and same-org bots", async () => {
  data.orgChannels = true;
  data.orgs = [{ id: "team", your_role: "admin" }];
  data.bots = [
    { id: "owned", user_id: "team", label: "Team bot", platform: "lark" },
    {
      id: "foreign",
      user_id: "different",
      label: "Other bot",
      platform: "lark",
    },
  ];
  const specialist = {
    ...agent("org-specialist", "idle", "org"),
    owner_id: "team",
  };
  render(<ChannelBotsManager agents={[agent("personal"), specialist]} />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Connect another bot" }));
  await user.click(
    screen.getByRole("combobox", { name: "Agent for Team bot" }),
  );
  expect(screen.getByRole("option", { name: "org-specialist" })).toBeVisible();
  await user.keyboard("{Escape}");
  await user.click(
    screen.getByRole("combobox", { name: "Agent for Other bot" }),
  );
  expect(
    screen.queryByRole("option", { name: "org-specialist" }),
  ).not.toBeInTheDocument();
});

it("keeps member verification available on an existing binding without giving management or linking rights", () => {
  data.orgs = [{ id: "team", your_role: "member" }];
  data.channels = [
    {
      ...connection("active"),
      org_specialist: {
        org_id: "team",
        guest_enabled: false,
        delegated_service_ids: [],
        daily_credits: null,
      },
    },
  ];
  render(
    <ChannelBotsManager
      agents={[{ ...agent("first", "idle", "org"), owner_id: "team" }]}
    />,
  );
  expect(screen.getByText("Member verification")).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Disconnect" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /Link account|Link another account/ }),
  ).not.toBeInTheDocument();
  expect(screen.getByTestId("channel-chat-agents")).toHaveAttribute("data-read-only", "true");
  expect(
    screen.getByRole("combobox", { name: "Agent for Receipts" }),
  ).toBeDisabled();
});


it.each(["discord.invalid_intents", undefined])("2183 explains working relay fallback and retries explicitly with detail %s", async (detail) => {
  data.channels = [nyxAgentChannelAgentSchema.parse({ ...connection("active"), platform: "discord", transport: "direct",
    gateway_native_activation: "gateway_rejected_request", gateway_native_activation_detail: detail,
    gateway_next_retry_at: "2026-11-05T00:00:00Z", gateway_retry_available_at: "2026-11-04T01:00:00Z",
    delivery_status: "ok" })];
  render(<ChannelBotsManager agents={[]} />);
  expect(screen.getByText("Working on NyxID relay.")).toBeVisible();
  expect(screen.getByText(`Native gateway activation did not complete: ${detail ?? "gateway_rejected_request"}.`)).toBeVisible();
  expect(screen.getByText(/Next automatic retry:/)).toBeVisible();
  expect(screen.getByText(/does not confirm native gateway acceptance/)).toBeVisible();
  await userEvent.setup().click(screen.getByRole("button", { name: "Retry native gateway now" }));
  expect(data.retryGateway).toHaveBeenCalledWith({ channelAgentId: "channel" });
});

it("2183 distinguishes native selection from confirmed delivery", () => {
  data.channels = [nyxAgentChannelAgentSchema.parse({ ...connection("active"), platform: "discord", transport: "gateway", gateway_source: { type: "discord", bot_id: "100" }, gateway_native_activation: "pending_discovery" })];
  render(<ChannelBotsManager agents={[]} />);
  expect(screen.getByText(/Native gateway selected.*pending_discovery/)).toBeVisible();
  expect(screen.getByText(/Native delivery has not been confirmed/)).toBeVisible();
  expect(screen.queryByText("Working on NyxID relay.")).toBeNull();
  expect(screen.queryByRole("button", { name: "Retry native gateway now" })).toBeNull();
});

it("2183 disables retry for an unverified owner and shows actionable refusal", async () => {
  const row = nyxAgentChannelAgentSchema.parse({ ...connection("active"), platform: "discord", transport: "direct", owner_linked: false, gateway_native_activation: "gateway_rejected_request" });
  data.channels = [row];
  const view = render(<ChannelBotsManager agents={[]} />);
  expect(screen.getByRole("button", { name: "Retry native gateway now" })).toBeDisabled();
  data.channels = [{ ...row, owner_linked: true }];
  view.rerender(<ChannelBotsManager agents={[]} />);
  data.retryGateway.mockRejectedValueOnce(new Error("native_gateway_retry_rate_limited"));
  await userEvent.setup().click(screen.getByRole("button", { name: "Retry native gateway now" }));
  expect(screen.getByText(/Wait ten minutes after the last gateway attempt/)).toBeVisible();
});

it("2183 never presents an older relay-sourced gateway as native", () => {
  data.channels = [nyxAgentChannelAgentSchema.parse({ ...connection("active"), platform: "discord", transport: "gateway" })];
  render(<ChannelBotsManager agents={[]} />);
  expect(screen.getByText(/Agent Event Gateway with NyxID relay intake/)).toBeVisible();
  expect(screen.queryByText(/Native gateway selected/)).toBeNull();
});

it("focuses the exact linked connection without starting member confirmation", () => {
  data.channels = [connection("active"), { ...connection("active"), id: "second", bot_label: "Other bot" }];
  render(<ChannelBotsManager agents={[agent("first")]} connectionId="second" />);
  expect(screen.getByRole("listitem", { name: "Other bot connection" })).toHaveFocus();
  expect(data.connect).not.toHaveBeenCalled();
  expect(data.link).not.toHaveBeenCalled();
});

it("explains individual verification after connecting an organization specialist", async () => {
  data.orgChannels = true;
  data.orgs = [{ id: "team", your_role: "admin" }];
  data.bots = [{ id: "owned", user_id: "team", label: "Team bot", platform: "lark" }];
  const specialist = { ...agent("org-specialist", "idle", "org"), owner_id: "team" };
  data.connect.mockResolvedValue({ channel_agent: { bot_label: "Team bot", status: "active", org_specialist: { org_id: "team" } } });
  render(<ChannelBotsManager agents={[specialist]} agent={specialist} />);
  await userEvent.click(screen.getByRole("button", { name: "Connect another bot" }));
  await userEvent.click(screen.getByRole("button", { name: "Connect Team bot to org-specialist" }));
  const disclosure = await screen.findByRole("dialog", {name:"Connect organization bot"});
  expect(disclosure).toHaveTextContent("organization pays");
  expect(disclosure).toHaveTextContent("Calendar: Use, but not change or delete");
  expect(disclosure).toHaveTextContent("Selected operations only");
  expect(disclosure).toHaveTextContent("Guests cannot use account tools");
  expect(data.connect).not.toHaveBeenCalled();
  await userEvent.click(within(disclosure).getByRole("button", {name:"Confirm connection"}));
  expect(await screen.findByText(/Each member, including admins, must verify their own chat account/)).toBeVisible();
  expect(screen.queryByText("LINK123")).not.toBeInTheDocument();
});


it("deep links to member verification within the matching organization connection", () => {
  const org = { org_id: "team", guest_enabled: false, delegated_service_ids: [] };
  data.channels = [{ ...connection("active"), org_specialist: org }, { ...connection("active"), id: "second", bot_label: "Other bot", org_specialist: org }];
  render(<ChannelBotsManager agents={[agent("first")]} connectionId="second" />);
  const row = screen.getByRole("listitem", { name: "Other bot connection" });
  expect(within(row).getByRole("region", { name: "Verify my chat account" })).toHaveFocus();
  expect(data.connect).not.toHaveBeenCalled();
});

it.each(["active", "pending", "failed"])("keeps the audience section mounted for %s bindings", (status) => {
  data.channels = [connection(status, status === "failed" ? "org_access_lost" : undefined)];
  render(<ChannelBotsManager agents={[agent("first")]} />);
  expect(screen.getByRole("region", { name: "Who can talk to this bot" })).toBeVisible();
});


it("shows intentional interactions-only intake without a delivery warning", () => {
  data.channels = [nyxAgentChannelAgentSchema.parse({
    id:"channel",channel_bot_id:"bot",platform:"discord",bot_label:"Receipts",transport:"direct",
    status:"active",owner_linked:true,delivery_status:"unknown",created_at:at,
    discord_intake:{receiving_side:"interactions_only",state:"not_enabled",code:null,block_reason:null,handed_over_at:null,last_inbound_at:null},
  })];
  render(<ChannelBotsManager agents={[]} />);
  expect(screen.getByText("Receiving side: Discord interactions. Intake: not_enabled.")).toBeVisible();
  expect(screen.queryByText(/Messages are not reaching|Intake not confirmed|Working on NyxID relay/)).not.toBeInTheDocument();
});


it.each(["unknown", "unconfirmed"])("does not call legacy relay status %s working", (status) => {
  data.channels = [nyxAgentChannelAgentSchema.parse({ ...connection("active"), platform:"discord",transport:"direct",
    gateway_native_activation:"gateway_rejected_request",delivery_status:status })];
  render(<ChannelBotsManager agents={[]} />);
  expect(screen.getByText("NyxID relay selected. Check channel health for delivery status.")).toBeVisible();
  expect(screen.queryByText("Working on NyxID relay.")).not.toBeInTheDocument();
  if (status === "unconfirmed") expect(screen.getByText(/Intake not confirmed: No authenticated inbound has been confirmed/)).toBeVisible();
  expect(screen.queryByText(/delivery failed/)).not.toBeInTheDocument();
});


it.each(["gateway_native_draining", "relay_draining"])("preserves intake evidence and the correct retry action for %s", (code) => {
  for (const intake of [undefined, { receiving_side: "nyxid_relay", state: "stopped", code: "discord_relay_blocked", block_reason: "auth_refused", handed_over_at: at, last_inbound_at: at }]) {
    data.channels = [nyxAgentChannelAgentSchema.parse({ ...connection("active"), platform: "discord", transport: "direct", gateway_native_activation: code, discord_intake: intake })];
    render(<ChannelBotsManager agents={[]} />);
    if (code === "gateway_native_draining") {
      expect(screen.getByText(/Discord intake is paused while/)).toBeVisible();
      expect(screen.queryByRole("button", { name: "Retry native gateway now" })).not.toBeInTheDocument();
    } else {
      expect(screen.queryByText(/Discord intake is paused while/)).not.toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Retry native gateway now" })).toBeVisible();
      if (!intake) expect(screen.getByText("Waiting for the native gateway to release Discord before the relay can resume.")).toBeVisible();
    }
    if (intake) {
      expect(screen.getByText("Receiving side: NyxID relay. Intake: stopped.")).toBeVisible();
      expect(screen.getByText(/^Last confirmed inbound:/)).toHaveTextContent(formatDateTime(at));
      expect(screen.getByText(/^Handed over:/)).toHaveTextContent(formatDateTime(at));
      expect(screen.getByText("Intake blocked by: auth_refused.")).toBeVisible();
    }
    expect(screen.queryByText(/activation did not complete|Working on NyxID relay/)).not.toBeInTheDocument();
    cleanup();
  }
});

it("renders the Discord release safety explanation in Chinese without a retry button", async () => {
  appI18n.addResourceBundle("zh-CN", "app", zh, true, true);
  await act(() => appI18n.changeLanguage("zh-CN"));
  try {
    data.channels = [nyxAgentChannelAgentSchema.parse({ ...connection("active"), platform: "discord", transport: "direct", gateway_native_activation: "gateway_native_draining" })];
    render(<ChannelBotsManager agents={[]} />);
    expect(screen.getByText(zh["Discord intake is paused while the retained gateway release and safety drain are checked. NyxID keeps checking; retrying activation cannot bypass this safety check."]!)).toBeVisible();
    expect(screen.queryByRole("button", { name: /重试|Retry native/ })).not.toBeInTheDocument();
  } finally { await act(() => appI18n.changeLanguage("en")); }
});

it.each([false, true])("returns registration to connection setup for the intended agent (org: %s)", async (org) => {
  const orgId = "12345678-1234-4123-8123-123456789012";
  const specialist = assistantAgentSchema.parse({ id: "specialist", name: "Research", kind: "specialist", status: "idle", owner_kind: org ? "org" : "person", owner_id: org ? orgId : "owner", can_maintain: true, created_at: "2026-10-01T00:00:00Z", last_active_at: "2026-10-01T00:00:00Z" });
  if (org) { data.orgChannels = true; data.orgs = [{ id: orgId, your_role: "admin" }]; }
  const view = render(<ChannelBotsManager agent={specialist} agents={[specialist]} />);
  await userEvent.click(screen.getByRole("button", { name: "Register a bot" }));
  expect(data.setupProps).toMatchObject({ defaultOrgId: org ? orgId : null, lockOrgScope: org, stayInPlace: true });
  data.bots = [{ id: "new-bot", label: "New helper", platform: "telegram", user_id: org ? orgId : "owner" }];
  await userEvent.click(screen.getByRole("button", { name: "Continue to connection setup" }));
  view.rerender(<ChannelBotsManager agent={specialist} agents={[specialist]} />);
  expect(data.refetchBots).toHaveBeenCalledOnce();
  expect(screen.queryByRole("region", { name: "Registration" })).toBeNull();
  expect(screen.getByText("Registered; not connected", { exact: false })).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Connect New helper to Research" }));
  expect(data.connect).toHaveBeenCalledWith({ botId: "new-bot", agentId: "specialist" });
});

it("cancels contextual registration without changing any connection", async () => {
  render(<ChannelBotsManager agents={[]} />);
  await userEvent.click(screen.getByRole("button", { name: "Register a bot" }));
  await userEvent.click(screen.getByRole("button", { name: "Cancel registration" }));
  expect(screen.getByRole("button", { name: "Connect another bot" })).toBeVisible();
  expect(data.connect).not.toHaveBeenCalled();
});
