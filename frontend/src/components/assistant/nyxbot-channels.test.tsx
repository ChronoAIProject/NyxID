import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { assistantAgentSchema, nyxAgentChannelAgentSchema, type AssistantAgent } from "@/schemas/assistant-nyxagent";
import { ChannelBotsManager } from "./nyxbot-channels";
import { ApiError } from "@/lib/api-client";

const data = vi.hoisted(() => ({
  channels: [] as unknown[],
  bots: [] as unknown[],
  orgs: [] as unknown[],
  orgChannels: false,
  connect: vi.fn(),
  link: vi.fn(),
  retryGateway: vi.fn(),
  pending: false,
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  nyxBotOf: (agents: AssistantAgent[]) => agents.find((agent) => agent.kind === "nyxbot"),
  useNyxBotChannels: () => ({ data: data.channels }),
  useConnectNyxBotChannel: () => ({ isPending: data.pending, mutateAsync: data.connect }),
  useDisconnectNyxBotChannel: () => ({ isPending: false }),
  useLinkNyxBotChannel: () => ({ isPending: false, mutateAsync: data.link }),
  useRetryNyxBotChannelGateway: () => ({ isPending: false, mutateAsync: data.retryGateway }),
}));
vi.mock("@/hooks/use-channel-bots", () => ({
  useChannelBots: () => ({ data: data.bots, isPending: false }),
}));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: data.orgs }) }));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => data.orgChannels,
}));
vi.mock("./org-specialist-channel-settings", () => ({
  OrgSpecialistChannelSettings: () => <section data-member-verification="" tabIndex={-1} aria-label="Verify my chat account">Member verification</section>,
}));
vi.mock("./nyxbot-channel-chats", () => ({
  ChannelChats: ({ agents }: { agents: readonly AssistantAgent[] }) => (
    <div data-testid="channel-chat-agents">
      {agents.map((agent) => <span key={agent.id}>{agent.name}</span>)}
    </div>
  ),
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
    "The Agent Event Gateway has not registered NyxBot as a provider yet (operator step). Telegram bots cannot use NyxBot until it is.",
  ],
  [
    "gateway_rejected_request",
    "The channel gateway rejected this connection request. Contact an administrator.",
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
  expect(screen.queryByTestId("channel-chat-agents")).not.toBeInTheDocument();
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
  expect(await screen.findByRole("status")).toHaveTextContent("Each member, including admins, must verify their own chat account");
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
