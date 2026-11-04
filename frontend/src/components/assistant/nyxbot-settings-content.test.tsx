import { confirmChanges } from "@/test-helpers/confirm-changes";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
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

vi.mock("@/lib/platform-personal", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform-personal")>()),
  loadPersonalBindings: vi.fn(),
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({ data: [], isPending: false, error: null }),
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
  useChannelBots: () => ({
    isPending: false,
    error: null,
    refetch: vi.fn(),
    data: [
      { id: "bot-tg", platform: "telegram", label: "Home bot" },
      { id: "bot-dc", platform: "discord", label: "Dev Notifications" },
    ],
  }),
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
  vi.mocked(loadPersonalBindings).mockReset().mockResolvedValue({
    bindings: [],
    next_before_id: null,
  });
  useAuthStore.getState().setUser({
    id: "owner",
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

function renderPage() {
  render(
    <QueryClientProvider client={client}>
      <Dialog open>
        <DialogContent scrollMode="body">
          <DialogTitle>NyxBot settings</DialogTitle>
          <DialogDescription>All agents</DialogDescription>
          <DialogBody>
            <NyxBotSettingsContent />
          </DialogBody>
        </DialogContent>
      </Dialog>
    </QueryClientProvider>,
  );
  return userEvent.setup({ delay: null });
}

it("exposes phone setup only after the server enables the adapter rollout", async () => {
  renderPage();
  expect(
    screen.queryByRole("region", { name: "Phone and speech connections" }),
  ).not.toBeInTheDocument();
  act(() =>
    useAuthStore.getState().setUser({
      ...useAuthStore.getState().user!,
      capabilities: { enabled_features: [FEATURE_FLAG.NYXBOT_ADAPTERS] },
    }),
  );
  expect(
    await screen.findByRole("region", { name: "Phone and speech connections" }),
  ).toBeInTheDocument();
});

// The legacy "Confirm destructive actions" switch is now the access mode:
// Auto (the default) confirms sensitive and destructive actions, Full access
// is the old "off" position and carries the same warning.
it("confirms destructive actions by default and saves only a changed access mode", async () => {
  const user = renderPage();
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const mode = await within(page).findByRole("combobox", {
    name: "Access mode",
  });
  expect(mode).toHaveTextContent("Auto");
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

  await user.click(mode);
  await user.click(await screen.findByRole("option", { name: "Full access" }));
  expect(mode).toHaveTextContent("Full access");
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
  expect(mode).toHaveTextContent("Full access");
});

it("asks before every change in Ask every time and explains the specialist request policy", async () => {
  const user = renderPage();
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const requests = await within(page).findByRole("combobox", {
    name: "Specialist access requests",
  });
  expect(requests).toHaveTextContent("Deny requests");
  expect(
    within(page).getByText(
      /A specialist you talk to directly tells you what it is missing and continues; you grant it in the specialist's details/,
    ),
  ).toBeVisible();
  // Denied mode: a direct chat is never handed to NyxBot; only assigned work reports there.
  expect(
    within(page).getByText(
      /Only work NyxBot assigned, or work in a group NyxBot is in, reports to NyxBot instead/,
    ),
  ).toBeVisible();
  await user.click(within(page).getByRole("combobox", { name: "Access mode" }));
  await user.click(
    await screen.findByRole("option", { name: "Ask every time" }),
  );
  expect(
    within(page).getByText(
      "Asks before every action that changes something. Reading never asks.",
    ),
  ).toBeVisible();
  await user.click(requests);
  await user.click(
    await screen.findByRole("option", { name: "Route to NyxBot" }),
  );
  expect(
    within(page).getByText(
      /For work NyxBot assigned, or in a group with NyxBot, a specialist's request goes to NyxBot, which approves or denies it/,
    ),
  ).toBeVisible();
  // Routed mode never claims that a direct chat with a specialist is handed to NyxBot.
  expect(
    within(page).getByText(
      /A specialist you talk to directly still tells you what it is missing/,
    ),
  ).toBeVisible();
  await user.click(within(page).getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() =>
    expect(writes.at(-1)?.body).toEqual({
      access_mode: "ask_every_time",
      specialist_access_requests: "routed_to_nyxbot",
    }),
  );
});

it("validates specialist limits against the server limits before saving", async () => {
  const user = renderPage();
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const live = await within(page).findByRole("spinbutton", {
    name: "Live specialists",
  });
  expect(live).toHaveValue(8);
  expect(within(page).getByText(/spend credits faster/)).toBeVisible();
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
  const user = renderPage();
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
  const user = renderPage();
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
  const user = renderPage();
  const page = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const connected = await within(page).findByRole("list", {
    name: "Connected channel bots",
  });
  const toggle = within(connected).getByRole("button", {
    name: "Chats and who can talk",
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
    within(page).getByRole("combobox", {
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

function settingsSection(title: string) {
  const heading = screen.getByRole("heading", { name: title });
  return within(heading.closest("section")!);
}

it("keeps Automations and Channel bots available while General loads", async () => {
  settingsRead = "pending";
  renderPage();
  expect(
    await settingsSection("General").findByRole("status"),
  ).toHaveTextContent("Loading settings...");
  expect(screen.getByRole("heading", { name: "General" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Automations" })).toBeVisible();
  expect(
    screen.getByRole("button", { name: "Timezone and automation budgets" }),
  ).toBeVisible();
  expect(
    await screen.findByRole("list", { name: "Connected channel bots" }),
  ).toHaveTextContent("Home bot");
  expect(
    screen.queryByRole("form", { name: "NyxBot preferences" }),
  ).not.toBeInTheDocument();
});

it("shows General's load error with retry without hiding the other sections", async () => {
  settingsRead = "error";
  vi.mocked(loadPersonalBindings).mockRejectedValue(
    new Error("Links unavailable"),
  );
  const user = renderPage();
  expect(
    await screen.findByText(
      "Could not load settings. NyxBot is temporarily unavailable. Try again shortly.",
    ),
  ).toBeVisible();
  expect(
    screen.getByRole("button", { name: "Timezone and automation budgets" }),
  ).toBeVisible();
  expect(
    await screen.findByRole("list", { name: "Connected channel bots" }),
  ).toHaveTextContent("Home bot");
  const personalLinks = screen.getByRole("region", {
    name: "Personal chat links",
  });
  expect(
    await within(personalLinks).findByText(
      "Personal chat links are unavailable.",
    ),
  ).toBeVisible();
  expect(
    within(personalLinks).getByRole("button", { name: "Retry" }),
  ).toBeVisible();
  settingsRead = "ready";
  await user.click(
    settingsSection("General").getByRole("button", { name: "Retry" }),
  );
  expect(
    await screen.findByRole("form", {
      name: "NyxBot preferences",
    }),
  ).toBeVisible();
  expect(vi.mocked(loadPersonalBindings)).toHaveBeenCalledTimes(1);
  expect(
    within(personalLinks).getByRole("button", { name: "Retry" }),
  ).toBeVisible();
});

it("keeps dirty values and reports a failed General save inline", async () => {
  saveError = true;
  const user = renderPage();
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
  const user = renderPage();
  const live = await screen.findByRole("spinbutton", {
    name: "Live specialists",
  });
  fireEvent.change(live, { target: { value: "12" } });
  await user.click(
    screen.getByRole("button", { name: "Timezone and automation budgets" }),
  );
  const hourly = screen.getByRole("spinbutton", { name: "Runs per hour" });
  fireEvent.change(hourly, { target: { value: "4" } });
  expect(
    screen.getAllByRole("region", { name: "Unsaved changes" }),
  ).toHaveLength(1);
  expect(
    screen.getByText("Unsaved: NyxBot preferences, automation preferences"),
  ).toBeInTheDocument();
  expect(
    [...document.forms].every((form) => !form.parentElement?.closest("form")),
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() =>
    expect(writes.map((write) => write.body)).toEqual([
      { max_live_subagents: 12 },
      {
        timezone: "UTC",
        schedule_minimum_minutes: 5,
        trigger_runs_per_hour: 4,
        trigger_runs_per_day: 300,
      },
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
  const user = renderPage();
  const live = await screen.findByRole("spinbutton", {
    name: "Live specialists",
  });
  fireEvent.change(live, { target: { value: "12" } });
  saveError = true;
  await user.click(
    screen.getByRole("button", { name: "Timezone and automation budgets" }),
  );
  const hourly = screen.getByRole("spinbutton", { name: "Runs per hour" });
  fireEvent.change(hourly, { target: { value: "4" } });
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() => expect(writes).toHaveLength(2));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "NyxBot is temporarily unavailable",
  );
  expect(live).toHaveValue(12);
  expect(screen.getByText("Unsaved: NyxBot preferences")).toBeInTheDocument();
  expect(
    screen.queryByRole("spinbutton", { name: "Runs per hour" }),
  ).not.toBeInTheDocument();
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
  renderPage();
  const connected = await screen.findByRole("list", {
    name: "Connected channel bots",
  });
  await user.click(
    within(connected).getByRole("button", { name: "Chats and who can talk" }),
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
  renderPage();
  postsError = true;
  const connected = await screen.findByRole("list", {
    name: "Connected channel bots",
  });
  await user.click(
    within(connected).getByRole("button", { name: "Chats and who can talk" }),
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
