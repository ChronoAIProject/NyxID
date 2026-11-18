import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import {
  GroupedDialog,
  GroupedDialogContent,
} from "@/components/ui/grouped-dialog";
import { withAccountPanelSearch } from "@/lib/assistant/account-panel-search";
import { useChannelBots } from "@/hooks/use-channel-bots";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { confirmChanges } from "@/test-helpers/confirm-changes";
import { DialogTitle, DialogDescription } from "@/components/ui/dialog";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { NyxBotSettingsContent } from "./nyxbot-settings-content";
import { loadPersonalBindings } from "@/lib/platform-personal";

vi.mock("@/hooks/use-nyxagent-live", () => ({ useNyxAgentLive: vi.fn() }));
const summaryState = vi.hoisted(() => ({
  fetching: false,
  triggers: [] as { id: string; status: string; delivery: { type: string } }[],
}));
vi.mock("@/hooks/use-triggers", () => ({
  useTriggers: () => ({
    data: { triggers: summaryState.triggers },
    isFetching: summaryState.fetching,
    isSuccess: true,
    isPending: false,
    error: null,
  }),
}));
vi.mock("@/lib/platform-personal", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform-personal")>()),
  loadPersonalBindings: vi.fn(),
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({ data: [], isPending: false, error: null }),
  useOrg: () => ({ data: undefined, isPending: false, error: null }),
}));

vi.mock("@/hooks/use-channel-adapters", () => ({
  useChannelAdapterCatalog: () => ({ data: [], error: null, refetch: vi.fn() }),
  useChannelAdapterBindings: () => ({
    data: [],
    isPending: false,
    isSuccess: true,
    error: null,
    refetch: vi.fn(),
  }),
}));

vi.mock("@/hooks/use-channel-bots", () => ({
  useChannelBots: vi.fn(() => ({
    isPending: false,
    isSuccess: true,
    error: null,
    refetch: vi.fn(),
    data: [
      { id: "bot-tg", platform: "telegram", label: "Home bot" },
      { id: "bot-dc", platform: "discord", label: "Dev Notifications", user_id: "org-1", owner_kind: "org" },
    ],
  })),
}));

const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });

let settingsRead: "ready" | "pending" | "error";
let saveError: boolean;
let settings: Record<string, unknown>;
let channelAgentId: string | null;
let privateChats: string;
let chats: Record<string, unknown>[];
const at = "2026-09-28T00:00:00Z";
function agent(
  id: string,
  kind: "nyxbot" | "specialist",
  name: string,
  status = "idle",
) {
  return {
    id,
    kind,
    name,
    description: "",
    specialty: null,
    created_by: "user",
    status,
    services: [],
    account_read: false,
    pending_requests: [],
    last_reply: null,
    home_conversation_id: null,
    memory_count: 0,
    created_at: at,
    last_active_at: at,
    destroyed_at: status === "destroyed" ? at : null,
    pending_acknowledgements: 0,
    channels: [],
  };
}
let privateChatPosts: boolean | undefined;
let postsError = false;
let writes: { method: string; endpoint: string; body: unknown }[];
let client: QueryClient;

beforeEach(() => {
  summaryState.triggers = [];
  summaryState.fetching = false;
  vi.mocked(loadPersonalBindings).mockReset().mockResolvedValue({
    bindings: [],
    next_before_id: null,
  });
  useAuthStore.getState().setUser({
    id: "owner",
    capabilities: { enabled_features: [FEATURE_FLAG.NYXAGENT_ENGINE, FEATURE_FLAG.ASSISTANT_NOTIFICATIONS] },
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-09-28T00:00:00Z",
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  writes = [];
  settingsRead = "ready";
  saveError = false;
  settings = {
    skip_destructive_confirmation: false,
    max_live_subagents: 8,
    max_concurrent_subagent_turns: 3,
    max_live_subagents_limit: 32,
    max_concurrent_subagent_turns_limit: 8,
  };
  channelAgentId = null;
  const channel = () => ({
    id: "channel-1",
    channel_bot_id: "bot-tg",
    platform: "telegram",
    bot_label: "Home bot",
    bot_username: "home_bot",
    transport: "gateway",
    status: "active",
    last_error: null,
    owner_linked: true,
    agent_id: channelAgentId,
    private_chats: privateChats,
    private_chat_posts: privateChatPosts,
    created_at: "2026-09-28T00:00:00Z",
  });
  privateChats = "owner";
  privateChatPosts = undefined;
  postsError = false;
  const chat = (id: string, kind: string, title: string) => ({
    id,
    channel_agent_id: "channel-1",
    platform: "telegram",
    bot_label: "Home bot",
    kind,
    title,
    agent_id: null,
    reply_mode: kind === "private" ? "all" : "mention",
    members: "everyone",
    members_setting: null,
    owner_seen: false,
    allow_posts: false,
    conversation_id: null,
    last_message_at: at,
  });
  chats = [
    chat("chat-g", "group", "Team chat"),
    chat("chat-p", "private", "Alice"),
  ];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const method = init.method ?? "GET";
    const body = init.body ? (JSON.parse(String(init.body)) as unknown) : null;
    if (method !== "GET") writes.push({ method, endpoint, body });
    if (endpoint.startsWith("/assistant/nyxagent/agents")) {
      return json({
        agents: [
          agent("agent-nyxbot", "nyxbot", "NyxBot"),
          agent("agent-writer", "specialist", "writer"),
          agent("agent-old", "specialist", "old", "destroyed"),
        ],
        limits: settings,
      });
    }
    if (endpoint === "/assistant/nyxagent/channels/channel-1/chats") {
      return json({ chats });
    }
    const chatPatch =
      /^\/assistant\/nyxagent\/channels\/channel-1\/chats\/([\w-]+)$/.exec(
        endpoint,
      );
    if (chatPatch && method === "PATCH") {
      const index = chats.findIndex((row) => row.id === chatPatch[1]);
      const update = body as Record<string, unknown>;
      chats[index] = {
        ...chats[index]!,
        ...update,
        ...(update.members ? { members_setting: update.members } : {}),
      };
      return json({ chat: chats[index] });
    }
    if (
      endpoint === "/assistant/nyxagent/channels/channel-1" &&
      method === "PATCH"
    ) {
      const update = body as {
        agent_id?: string;
        private_chats?: string;
        private_chat_posts?: boolean;
        apply_to_existing?: boolean;
      };
      if (update.private_chat_posts !== undefined) {
        if (postsError) return json({ message: "Could not save posting" }, 503);
        privateChatPosts = update.private_chat_posts;
        if (update.apply_to_existing)
          chats = chats.map((chat) =>
            chat.kind === "private"
              ? { ...chat, allow_posts: privateChatPosts! }
              : chat,
          );
        return json({
          channel_agent_id: "channel-1",
          private_chat_posts: privateChatPosts,
        });
      }
      if (update.private_chats) {
        privateChats = update.private_chats;
        return json({
          channel_agent_id: "channel-1",
          private_chats: privateChats,
        });
      }
      channelAgentId = update.agent_id ?? null;
      return json({
        channel_agent_id: "channel-1",
        agent: "writer",
        changed: true,
      });
    }
    if (endpoint === "/assistant/nyxagent/settings") {
      if (method === "GET" && settingsRead === "pending")
        return new Promise<Response>(() => {});
      if (method === "GET" && settingsRead === "error")
        return json({ message: "Settings unavailable" }, 503);
      if (
        method === "PUT" &&
        saveError &&
        (body as Record<string, unknown>).max_live_subagents !== undefined
      )
        return json({ message: "Save unavailable" }, 503);
      if (method === "PUT") settings = { ...settings, ...(body as object) };
      return json(settings);
    }
    if (endpoint === "/assistant/nyxagent/channels") {
      if (method === "POST") {
        return json({
          channel_agent: {
            ...channel(),
            id: "channel-2",
            channel_bot_id: "bot-dc",
            platform: "discord",
            bot_label: "Dev Notifications",
            bot_username: null,
            owner_linked: false,
          },
          link: {
            code: "nyxlink_abc234",
            url: null,
            expires_at: "2026-09-29T00:00:00Z",
            instructions:
              "Send this code to the bot once from your own discord account: nyxlink_abc234",
          },
        });
      }
      return json({ channel_agents: [channel()] });
    }
    return json({ message: "not found" }, 404);
  };
});

afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
});

let panelRouter: ReturnType<typeof createRouter>;
function renderPage(section?: string) {
  const root = createRootRoute();
  const route = createRoute({
    getParentRoute: () => root,
    path: "/assistant",
    validateSearch: withAccountPanelSearch(() => ({})),
    component: () => (
      <GroupedDialog open>
        <GroupedDialogContent scrollMode="body">
          <DialogTitle className="sr-only">NyxBot settings</DialogTitle>
          <DialogDescription>All agents</DialogDescription>
          <NyxBotSettingsContent />
        </GroupedDialogContent>
      </GroupedDialog>
    ),
  });
  panelRouter = createRouter({
    routeTree: root.addChildren([route]),
    history: createMemoryHistory({
      initialEntries: [
        `/assistant?panel=nyxbot${section ? `&panelTab=${encodeURIComponent(section)}` : ""}`,
      ],
    }),
  });
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={panelRouter} />
    </QueryClientProvider>,
  );
  return userEvent.setup({ delay: null });
}
async function go(section?: string) {
  await act(() =>
    panelRouter.navigate({
      to: "/assistant",
      search: { panel: "nyxbot", panelTab: section },
      replace: true,
    }),
  );
}

it("exposes phone setup only after the server enables the adapter rollout", async () => {
  const user = renderPage("channels");
  await screen.findByRole("heading", { name: "Chats & channels" });
  expect(
    screen.queryByRole("button", { name: "Adapter bindings" }),
  ).not.toBeInTheDocument();
  act(() =>
    useAuthStore.getState().setUser({
      ...useAuthStore.getState().user!,
      capabilities: { enabled_features: [FEATURE_FLAG.NYXBOT_ADAPTERS] },
    }),
  );
  await user.click(
    await screen.findByRole("button", { name: "Adapter bindings" }),
  );
  expect(
    await screen.findByRole("region", { name: "Phone and speech connections" }),
  ).toBeInTheDocument();
});

// The legacy "Confirm destructive actions" switch is now the access mode:
// Auto (the default) confirms sensitive and destructive actions, Full access
// is the old "off" position and carries the same warning.
it("confirms destructive actions by default and saves only a changed access mode", async () => {
  const user = renderPage("permissions");
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const mode = await within(page).findByRole("radio", { name: /^Auto / });
  expect(mode).toHaveAttribute("aria-checked", "true");
  expect(
    within(page).getByText(
      "Acts on its own; asks before sensitive or destructive actions.",
    ),
  ).toBeVisible();
  expect(
    within(page).queryByRole("button", { name: "Save changes" }),
  ).not.toBeInTheDocument();
  expect(
    within(page).queryByText(/without asking you first/),
  ).not.toBeInTheDocument();

  const full = screen.getByRole("radio", { name: /^Full access / });
  await user.click(full);
  expect(full).toHaveAttribute("aria-checked", "true");
  expect(
    within(page).getByText(
      /deletes keys, channel bots, services and\s+nodes and changes access without asking you first/,
    ),
  ).toBeVisible();
  const save = within(page).getByRole("button", { name: "Save changes" });
  expect(save).toBeEnabled();
  await user.click(save);
  await confirmChanges();
  await waitFor(() =>
    expect(writes).toEqual([
      {
        method: "PUT",
        endpoint: "/assistant/nyxagent/settings",
        body: { access_mode: "autopilot" },
      },
    ]),
  );
  await waitFor(() =>
    expect(
      within(page).queryByRole("button", { name: "Save changes" }),
    ).not.toBeInTheDocument(),
  );
  expect(full).toHaveAttribute("aria-checked", "true");
});

it("edits both permission groups through one reviewed save", async () => {
  const user = renderPage("permissions");
  await user.click(
    await screen.findByRole("radio", { name: /^Ask every time / }),
  );
  expect(
    screen.getByText(
      /Only work NyxBot assigned, or work in a group NyxBot is in, reports to NyxBot instead/,
    ),
  ).toBeVisible();
  await user.click(screen.getByRole("radio", { name: /^Route to NyxBot / }));
  expect(
    screen
      .getByRole("button", { name: "Save changes" })
      .closest("[inert], [aria-hidden=true]"),
  ).toBeNull();
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() =>
    expect(writes.map((write) => write.body)).toEqual([
      { access_mode: "ask_every_time", specialist_access_requests: "routed_to_nyxbot" },
    ]),
  );
});

it("validates specialist limits against the server limits before saving", async () => {
  const user = renderPage("tasks");
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const live = await within(page).findByRole("spinbutton", {
    name: "Live specialists",
  });
  expect(live).toHaveValue(8);
  expect(within(page).getAllByText(/spend credits faster/).some(node => !node.closest("[hidden], [aria-hidden=true]"))).toBe(true);
  await user.clear(live);
  await user.type(live, "40");
  await user.click(within(page).getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  expect(await within(page).findByText("Must be at most 32")).toBeVisible();
  expect(writes).toEqual([]);
  fireEvent.change(live, { target: { value: "12" } });
  await user.click(within(page).getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() =>
    expect(writes.at(-1)?.body).toEqual({ max_live_subagents: 12 }),
  );
});

it("saves group hand-off limits within the server's bounds", async () => {
  const user = renderPage("tasks");
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const perMessage = await within(page).findByRole("spinbutton", {
    name: "Group hand-offs per message",
  });
  const perHour = within(page).getByRole("spinbutton", {
    name: "Group hand-offs per hour",
  });
  // Older servers omit the fields: the defaults apply.
  expect(perMessage).toHaveValue(6);
  expect(perHour).toHaveValue(60);
  await user.clear(perMessage);
  await user.type(perMessage, "30");
  await user.click(within(page).getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  expect(await within(page).findByText("Must be at most 24")).toBeVisible();
  expect(writes).toEqual([]);
  await user.clear(perMessage);
  await user.type(perMessage, "0");
  await user.clear(perHour);
  await user.type(perHour, "120");
  await user.click(within(page).getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() =>
    expect(writes.at(-1)?.body).toEqual({
      max_group_handoffs: 0,
      max_group_handoffs_per_hour: 120,
    }),
  );
});

it("lists connected bots with their agent, relinks one, and connects another to a chosen agent", async () => {
  const user = renderPage("channels");
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const connected = await within(page).findByRole("list", {
    name: "Connected channel bots",
  });
  expect(connected).toHaveTextContent("Home bot");
  expect(connected).toHaveTextContent("Telegram");
  expect(connected).toHaveTextContent("Owner linked");
  // A bot without an agent reaches NyxBot.
  const agentFor = within(connected).getByRole("combobox", {
    name: "Agent for Home bot",
  });
  await waitFor(() => expect(agentFor).toHaveTextContent("NyxBot"));
  await user.click(agentFor);
  // Destroyed agents cannot receive a bot.
  expect(screen.queryByRole("option", { name: "old" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("option", { name: "writer" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/channels/channel-1",
      body: { agent_id: "agent-writer" },
    }),
  );
  await waitFor(() => expect(agentFor).toHaveTextContent("writer"));

  // A connected bot is not offered again; the other connects to the chosen agent.
  await user.click(
    within(page).getByRole("button", { name: "Connect another bot" }),
  );
  const available = within(page).getByRole("list", {
    name: "Channel bots you can connect",
  });
  expect(available).not.toHaveTextContent("Home bot");
  await user.click(
    within(available).getByRole("combobox", {
      name: "Agent for Dev Notifications",
    }),
  );
  await user.click(screen.getByRole("option", { name: "writer" }));
  await user.click(
    within(available).getByRole("button", {
      name: "Connect Dev Notifications",
    }),
  );
  const link = await within(page).findByRole("region", { name: "Owner link" });
  expect(link).toHaveTextContent("nyxlink_abc234");
  expect(link).toHaveTextContent("Send this code to the bot");
  // No URL means no link to open.
  expect(within(link).queryByRole("link")).not.toBeInTheDocument();
  expect(writes).toContainEqual({
    method: "POST",
    endpoint: "/assistant/nyxagent/channels",
    body: { bot_id: "bot-dc", agent_id: "agent-writer" },
  });
  await user.click(within(link).getByRole("button", { name: "New link" }));
  await waitFor(() =>
    expect(writes.filter((write) => write.method === "POST")).toHaveLength(2),
  );
  expect(
    writes.filter((write) => write.method === "POST").at(-1)?.body,
  ).toEqual({
    bot_id: "bot-dc",
    agent_id: "agent-writer",
  });
});

it("opens a bot's chats and saves who can talk and how it answers", async () => {
  const user = renderPage("channels");
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const connected = await within(page).findByRole("list", {
    name: "Connected channel bots",
  });
  const toggle = within(connected).getByRole("button", {
    name: "Chats and posting",
  });
  await user.click(toggle);
  expect(
    within(page).getByRole("heading", { name: "Home bot · Chats" }),
  ).toHaveFocus();
  const list = await within(page).findByRole("list", {
    name: "Chats of Home bot",
  });
  // Groups have reply and member settings; private chats do not.
  expect(
    within(list).getByRole("combobox", { name: "Replies in Team chat" }),
  ).toHaveTextContent("When mentioned");
  expect(
    within(list).queryByRole("combobox", { name: "Replies in Alice" }),
  ).not.toBeInTheDocument();
  await user.click(
    within(list).getByRole("combobox", { name: "Replies in Team chat" }),
  );
  await user.click(screen.getByRole("option", { name: "Every message" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/channels/channel-1/chats/chat-g",
      body: { reply_mode: "all" },
    }),
  );
  // Before the user has talked there, only they can; they can pin that.
  const members = within(list).getByRole("combobox", {
    name: "Who can talk in Team chat",
  });
  expect(members).toHaveTextContent("You, until you talk here");
  await user.click(members);
  await user.click(screen.getByRole("option", { name: "Only you" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/channels/channel-1/chats/chat-g",
      body: { members: "owner" },
    }),
  );
  await user.click(
    within(list).getByRole("switch", {
      name: "Let the agent post in Team chat on its own",
    }),
  );
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/channels/channel-1/chats/chat-g",
      body: { allow_posts: true },
    }),
  );
  await user.click(
    within(page).getByRole("button", { name: "Back to Channels" }),
  );
  const audience = within(page).getByRole("region", {
    name: "Who can talk to this bot · Home bot",
  });
  expect(audience).toBeVisible();
  await user.click(
    within(audience).getByRole("combobox", {
      name: "Who can talk to Home bot in private chats",
    }),
  );
  await user.click(screen.getByRole("option", { name: "Anyone" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/channels/channel-1",
      body: { private_chats: "everyone" },
    }),
  );
});

it("keeps root categories available with no guessed summaries while settings load", async () => {
  settingsRead = "pending";
  const user = renderPage();
  const permissions = await screen.findByRole("button", {
    name: "Permissions",
  });
  expect(permissions).not.toHaveTextContent("Auto");
  expect(screen.getByRole("button", { name: "Automations 0" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Tasks & specialists" }));
  expect(await screen.findByRole("status")).toHaveTextContent(
    "Loading settings...",
  );
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  await user.click(screen.getByRole("button", { name: "Chats & channels 2" }));
  expect(
    await screen.findByRole("list", { name: "Connected channel bots" }),
  ).toHaveTextContent("Home bot");
});

it("shows load errors with retry and retains independent channel and personal-link pages", async () => {
  settingsRead = "error";
  vi.mocked(loadPersonalBindings).mockRejectedValue(
    new Error("Links unavailable"),
  );
  const user = renderPage("permissions");
  expect((await screen.findAllByText(/Could not load settings/)).find(element => !element.closest("[hidden]"))).toBeVisible();
  settingsRead = "ready";
  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByRole("radio", { name: /^Auto / })).toBeVisible();
  await go("channels/links");
  expect(
    await screen.findByText("Personal chat links are unavailable."),
  ).toBeVisible();
});

it("keeps dirty values and reports a failed General save inline", async () => {
  saveError = true;
  const user = renderPage("tasks");
  const live = await screen.findByRole("spinbutton", {
    name: "Live specialists",
  });
  fireEvent.change(live, { target: { value: "12" } });
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "NyxBot is temporarily unavailable. Try again shortly.",
  );
  expect(live).toHaveValue(12);
  expect(screen.getByRole("button", { name: "Save changes" })).toBeEnabled();
});

it("saves dirty General and Automation forms in order through one bar with independent payloads", async () => {
  settings.timezone = "UTC";
  const user = renderPage("tasks");
  const live = await screen.findByRole("spinbutton", {
    name: "Live specialists",
  });
  fireEvent.change(live, { target: { value: "12" } });
  await go("automations");
  await screen.findByRole("spinbutton", { name: "Runs per hour" });
  const hourly = screen.getByRole("spinbutton", { name: "Runs per hour" });
  fireEvent.change(hourly, { target: { value: "4" } });
  expect(
    screen.getAllByRole("region", { name: "Unsaved changes" }),
  ).toHaveLength(1);
  expect(
    screen.getByText("Unsaved: Tasks & specialists, automation preferences"),
  ).toBeInTheDocument();
  expect(
    [...document.forms].every((form) => !form.parentElement?.closest("form")),
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() =>
    expect(writes.map((write) => write.body)).toEqual([
      { max_live_subagents: 12 },
      { trigger_runs_per_hour: 4 },
    ]),
  );
  await waitFor(() =>
    expect(
      screen.queryByRole("region", { name: "Unsaved changes" }),
    ).not.toBeInTheDocument(),
  );
});

it("keeps failed General changes dirty while Automation preferences succeed", async () => {
  settings.timezone = "UTC";
  const user = renderPage("tasks");
  const live = await screen.findByRole("spinbutton", {
    name: "Live specialists",
  });
  fireEvent.change(live, { target: { value: "12" } });
  saveError = true;
  await go("automations");
  await screen.findByRole("spinbutton", { name: "Runs per hour" });
  const hourly = screen.getByRole("spinbutton", { name: "Runs per hour" });
  fireEvent.change(hourly, { target: { value: "4" } });
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() => expect(writes).toHaveLength(2));
  await go("tasks");
  expect(await screen.findByRole("alert")).toHaveTextContent("NyxBot is temporarily unavailable");
  await go("automations");
  expect(live).toHaveValue(12);
  expect(screen.getByText("Unsaved: Tasks & specialists")).toBeInTheDocument();
  expect(screen.getByRole("spinbutton", { name: "Runs per hour" })).toHaveValue(
    4,
  );
  saveError = false;
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() =>
    expect(
      screen.queryByRole("region", { name: "Unsaved changes" }),
    ).not.toBeInTheDocument(),
  );
});

it("defaults new private chat posting off on legacy responses and saves without changing existing chats", async () => {
  const user = userEvent.setup({ delay: null });
  renderPage("channels");
  const connected = await screen.findByRole("list", {
    name: "Connected channel bots",
  });
  await user.click(
    within(connected).getByRole("button", { name: "Chats and posting" }),
  );
  const toggle = screen.getByRole("switch", {
    name: "Allow posts in new private chats with Home bot",
  });
  expect(toggle).not.toBeChecked();
  await user.click(toggle);
  await waitFor(() => expect(toggle).toBeChecked());
  expect(writes).toContainEqual({
    method: "PATCH",
    endpoint: "/assistant/nyxagent/channels/channel-1",
    body: { private_chat_posts: true, apply_to_existing: false },
  });
  await user.click(
    screen.getByRole("switch", {
      name: "Also apply to existing private chats",
    }),
  );
  await user.click(
    screen.getByRole("button", { name: "Apply current setting" }),
  );
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/channels/channel-1",
      body: { private_chat_posts: true, apply_to_existing: true },
    }),
  );
  await user.click(toggle);
  await waitFor(() => expect(toggle).not.toBeChecked());
  expect(writes).toContainEqual({
    method: "PATCH",
    endpoint: "/assistant/nyxagent/channels/channel-1",
    body: { private_chat_posts: false, apply_to_existing: false },
  });
});
it("keeps the posting switch unchanged and exposes a save failure", async () => {
  const user = userEvent.setup({ delay: null });
  renderPage("channels");
  postsError = true;
  const connected = await screen.findByRole("list", {
    name: "Connected channel bots",
  });
  await user.click(
    within(connected).getByRole("button", { name: "Chats and posting" }),
  );
  const toggle = screen.getByRole("switch", {
    name: "Allow posts in new private chats with Home bot",
  });
  await user.click(toggle);
  await waitFor(() =>
    expect(screen.getByRole("alert")).toHaveTextContent(
      "NyxBot is temporarily unavailable",
    ),
  );
  expect(toggle).not.toBeChecked();
});

it("normalizes a removed nested task page and returns focus to the root row", async () => {
  const user = renderPage("tasks/specialists");
  const heading = await screen.findByRole("heading", { name: "Tasks & specialists", level: 2 });
  await waitFor(() => expect(heading).toHaveFocus());
  expect(panelRouter.state.location.search).toEqual({ panel: "nyxbot", panelTab: "tasks" });
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  expect(screen.getByRole("button", { name: "Tasks & specialists" })).toHaveFocus();
});

it("falls back from disabled adapters to channels and replaces the URL", async () => {
  renderPage("channels/adapters");
  expect(
    await screen.findByRole("heading", { name: "Chats & channels", level: 2 }),
  ).toBeVisible();
  await waitFor(() =>
    expect(panelRouter.state.location.search).toEqual({
      panel: "nyxbot",
      panelTab: "channels",
    }),
  );
  expect(new URLSearchParams(panelRouter.state.location.searchStr).get("panelTab")).toBe("channels");
  expect(
    screen.queryByRole("button", { name: "Adapter bindings" }),
  ).not.toBeInTheDocument();
});

it("retains a dirty draft on refetch errors and applies live values to pristine fields", async () => {
  renderPage("tasks");
  const live = await screen.findByRole("spinbutton", {
    name: "Live specialists",
  });
  fireEvent.change(live, { target: { value: "12" } });
  act(() =>
    client.setQueryData(nyxBotQueryKeys.settings("owner"), {
      ...client.getQueryData(nyxBotQueryKeys.settings("owner"))!,
      max_live_subagents: 20,
      max_concurrent_subagent_turns: 5,
    }),
  );
  expect(live).toHaveValue(12);
  await waitFor(() => expect(screen.getByRole("spinbutton", { name: "Specialists working at once" })).toHaveValue(5));
  settingsRead = "error";
  await act(() =>
    client.invalidateQueries({ queryKey: nyxBotQueryKeys.settings("owner") }),
  );
  expect((await screen.findAllByText(/Could not load settings/)).find(element => !element.closest("[hidden]"))).toBeVisible();
  expect(live).toHaveValue(12);
  await go();
  expect(
    screen.getByRole("button", { name: "Permissions" }),
  ).not.toHaveTextContent("Auto");
  await go("tasks");
  expect(
    screen.getByRole("spinbutton", { name: "Live specialists" }),
  ).toHaveValue(12);
});

it("guards dirty Back once and retains the draft when editing continues", async () => {
  const user = renderPage("tasks");
  const live = await screen.findByRole("spinbutton", {
    name: "Live specialists",
  });
  fireEvent.change(live, { target: { value: "12" } });
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  const guard = await screen.findByRole("dialog", {
    name: "Discard unsaved changes?",
  });
  await user.click(within(guard).getByRole("button", { name: "Keep editing" }));
  expect(live).toHaveValue(12);
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  await user.click(
    within(await screen.findByRole("dialog", { name: "Discard unsaved changes?" })).getByRole("button", {
      name: "Discard",
    }),
  );
  await waitFor(() =>
    expect(panelRouter.state.location.search).toEqual({
      panel: "nyxbot",
    }),
  );
  expect(screen.queryByRole("dialog", { name: "Discard unsaved changes?" })).not.toBeInTheDocument();
});


it("summarizes saved mode, active assistant automations, push kinds and bot count", async () => {
  settings.notifications = { push: { automation_result: true, automation_problem: false, needs_decision: true, agent_update: false, chat_update: false, group_mention: false } };
  summaryState.triggers = [
    { id: "active", status: "active", delivery: { type: "assistant" } },
    { id: "paused", status: "paused", delivery: { type: "assistant" } },
    { id: "webhook", status: "active", delivery: { type: "webhook" } },
  ];
  renderPage();
  expect(await screen.findByRole("button", { name: "Permissions Auto" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Automations 1" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Notifications 2 on" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Chats & channels 2" })).toBeVisible();
});

it("keeps loaded automation summaries during a background refresh", async () => {
  summaryState.fetching = true;
  summaryState.triggers = [{ id: "active", status: "active", delivery: { type: "assistant" } }];
  renderPage();
  expect(await screen.findByRole("button", { name: "Automations 1" })).toHaveTextContent("1");
});


it("keeps the saved automation summary during a refetch and shows Off for disabled push", async () => {
  summaryState.fetching = true;
  settings.notifications = { push: { automation_result: false, automation_problem: false, needs_decision: false, agent_update: false, chat_update: false, group_mention: false } };
  renderPage();
  expect(await screen.findByRole("button", { name: "Notifications Off" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Automations 0" })).toBeVisible();
});

it.each(["unknown", "tasks/", "[\"tasks\"]"])("normalizes invalid section %s to the root URL", async (section) => {
  renderPage(section);
  await waitFor(() => expect(panelRouter.state.location.search).toEqual({ panel: "nyxbot" }));
  await waitFor(() => expect(new URLSearchParams(panelRouter.state.location.searchStr).has("panelTab")).toBe(false));
  expect(screen.getByRole("button", { name: "Tasks & specialists" })).toBeVisible();
});


it("hides notifications and replaces its unavailable deep link with the root", async () => {
  const owner = useAuthStore.getState().user!;
  useAuthStore.getState().setUser({ ...owner, capabilities: { enabled_features: [FEATURE_FLAG.NYXAGENT_ENGINE] } });
  renderPage("notifications");
  await waitFor(() => expect(panelRouter.state.location.search).toEqual({ panel: "nyxbot" }));
  expect(new URLSearchParams(panelRouter.state.location.searchStr).has("panelTab")).toBe(false);
  expect(screen.queryByRole("button", { name: /^Notifications/ })).not.toBeInTheDocument();
});

it("uses named radio groups with keyboard selection", async () => {
  const user = renderPage("permissions");
  const auto = await screen.findByRole("radio", { name: /^Auto / });
  expect(screen.getByRole("radiogroup", { name: "Access mode" })).toBeVisible();
  let group = screen.getByRole("radiogroup", { name: "Access mode" });
  const tabStops = () => within(group).getAllByRole("radio").filter(row => row.tabIndex === 0);
  expect(tabStops()).toEqual([auto]);
  expect(within(group).getAllByRole("radio").filter(row => row !== auto).every(row => row.tabIndex === -1)).toBe(true);
  auto.focus();
  await user.keyboard("{ArrowUp}");
  expect(screen.getByRole("radio", { name: /^Full access / })).toHaveAttribute("aria-checked", "true");
  expect(tabStops()).toEqual([screen.getByRole("radio", { name: /^Full access / })]);
  await go("permissions");
  group = screen.getByRole("radiogroup", { name: "Specialist access requests" });
  const deny = within(group).getByRole("radio", { name: /^Deny requests / });
  expect(screen.getByRole("radiogroup", { name: "Specialist access requests" })).toBeVisible();
  expect(tabStops()).toEqual([deny]);
  deny.focus();
  await user.keyboard("{End}");
  expect(screen.getByRole("radio", { name: /^Route to NyxBot / })).toHaveFocus();
  expect(screen.getByRole("radio", { name: /^Route to NyxBot / })).toHaveAttribute("aria-checked", "true");
  expect(tabStops()).toEqual([screen.getByRole("radio", { name: /^Route to NyxBot / })]);
});

it("counts the same all-scope inventory, including organisation bots, shown in Channels", async () => {
  const user = renderPage();
  await user.click(await screen.findByRole("button", { name: "Chats & channels 2" }));
  expect(vi.mocked(useChannelBots)).toHaveBeenCalledWith({ scope: "all" });
  expect(await screen.findByRole("listitem", { name: "Home bot connection" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Connect another bot" }));
  expect(screen.getByText("Dev Notifications", { exact: true })).toBeVisible();
});
