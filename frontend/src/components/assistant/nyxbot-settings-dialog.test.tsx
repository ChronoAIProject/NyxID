import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { NyxBotSettingsButton } from "./nyxbot-settings-dialog";

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({ data: [], isPending: false, error: null }),
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

let settings: Record<string, unknown>;
let channelAgentId: string | null;
let privateChats: string;
let chats: Record<string, unknown>[];
const at = "2026-09-28T00:00:00Z";
function agent(id: string, kind: "nyxbot" | "specialist", name: string, status = "idle") {
  return {
    id, kind, name, description: "", specialty: null, created_by: "user", status,
    services: [], account_read: false, pending_requests: [], last_reply: null,
    home_conversation_id: null, memory_count: 0, created_at: at, last_active_at: at,
    destroyed_at: status === "destroyed" ? at : null, pending_acknowledgements: 0, channels: [],
  };
}
let writes: { method: string; endpoint: string; body: unknown }[];
let client: QueryClient;

beforeEach(() => {
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
    created_at: "2026-09-28T00:00:00Z",
  });
  privateChats = "owner";
  const chat = (id: string, kind: string, title: string) => ({
    id, channel_agent_id: "channel-1", platform: "telegram", bot_label: "Home bot", kind, title,
    agent_id: null, reply_mode: kind === "private" ? "all" : "mention", members: "everyone",
    allow_posts: false, conversation_id: null, last_message_at: at,
  });
  chats = [chat("chat-g", "group", "Team chat"), chat("chat-p", "private", "Alice")];
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
    const chatPatch = /^\/assistant\/nyxagent\/channels\/channel-1\/chats\/([\w-]+)$/.exec(
      endpoint,
    );
    if (chatPatch && method === "PATCH") {
      const index = chats.findIndex((row) => row.id === chatPatch[1]);
      chats[index] = { ...chats[index]!, ...(body as object) };
      return json({ chat: chats[index] });
    }
    if (endpoint === "/assistant/nyxagent/channels/channel-1" && method === "PATCH") {
      const update = body as { agent_id?: string; private_chats?: string };
      if (update.private_chats) {
        privateChats = update.private_chats;
        return json({ channel_agent_id: "channel-1", private_chats: privateChats });
      }
      channelAgentId = update.agent_id ?? null;
      return json({ channel_agent_id: "channel-1", agent: "writer", changed: true });
    }
    if (endpoint === "/assistant/nyxagent/settings") {
      if (method === "PUT") settings = { ...settings, ...(body as object) };
      return json(settings);
    }
    if (endpoint === "/assistant/nyxagent/channels") {
      if (method === "POST") {
        return json({
          channel_agent: { ...channel(), id: "channel-2", channel_bot_id: "bot-dc", platform: "discord", bot_label: "Dev Notifications", bot_username: null, owner_linked: false },
          link: {
            code: "nyxlink_abc234",
            url: null,
            expires_at: "2026-09-29T00:00:00Z",
            instructions: "Send this code to the bot once from your own discord account: nyxlink_abc234",
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

function renderButton() {
  render(
    <QueryClientProvider client={client}>
      <NyxBotSettingsButton />
    </QueryClientProvider>,
  );
  return userEvent.setup();
}

it("confirms destructive actions by default and saves only a changed toggle", async () => {
  const user = renderButton();
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  const dialog = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const toggle = await within(dialog).findByRole("switch", {
    name: "Confirm destructive actions",
  });
  expect(toggle).toHaveAttribute("aria-checked", "true");
  const save = within(dialog).getByRole("button", { name: "Save settings" });
  expect(save).toBeDisabled();
  expect(within(dialog).queryByText(/without asking you first/)).not.toBeInTheDocument();

  await user.click(toggle);
  expect(toggle).toHaveAttribute("aria-checked", "false");
  expect(within(dialog).getByText(/will delete keys, channel bots, services and nodes without asking/)).toBeVisible();
  expect(save).toBeEnabled();
  await user.click(save);
  await waitFor(() =>
    expect(writes).toEqual([
      {
        method: "PUT",
        endpoint: "/assistant/nyxagent/settings",
        body: { skip_destructive_confirmation: true },
      },
    ]),
  );
  await waitFor(() => expect(save).toBeDisabled());
  expect(toggle).toHaveAttribute("aria-checked", "false");
});

it("validates specialist limits against the server limits before saving", async () => {
  const user = renderButton();
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  const dialog = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const live = await within(dialog).findByRole("spinbutton", { name: "Live specialists" });
  expect(live).toHaveValue(8);
  expect(within(dialog).getByText(/spend credits faster/)).toBeVisible();
  await user.clear(live);
  await user.type(live, "40");
  await user.click(within(dialog).getByRole("button", { name: "Save settings" }));
  expect(await within(dialog).findByText("Must be at most 32")).toBeVisible();
  expect(writes).toEqual([]);
  await user.clear(live);
  await user.type(live, "12");
  await user.click(within(dialog).getByRole("button", { name: "Save settings" }));
  await waitFor(() =>
    expect(writes.at(-1)?.body).toEqual({ max_live_subagents: 12 }),
  );
});

it("saves group hand-off limits within the server's bounds", async () => {
  const user = renderButton();
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  const dialog = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const perMessage = await within(dialog).findByRole("spinbutton", {
    name: "Group hand-offs per message",
  });
  const perHour = within(dialog).getByRole("spinbutton", { name: "Group hand-offs per hour" });
  // Older servers omit the fields: the defaults apply.
  expect(perMessage).toHaveValue(6);
  expect(perHour).toHaveValue(60);
  await user.clear(perMessage);
  await user.type(perMessage, "30");
  await user.click(within(dialog).getByRole("button", { name: "Save settings" }));
  expect(await within(dialog).findByText("Must be at most 24")).toBeVisible();
  expect(writes).toEqual([]);
  await user.clear(perMessage);
  await user.type(perMessage, "0");
  await user.clear(perHour);
  await user.type(perHour, "120");
  await user.click(within(dialog).getByRole("button", { name: "Save settings" }));
  await waitFor(() =>
    expect(writes.at(-1)?.body).toEqual({ max_group_handoffs: 0, max_group_handoffs_per_hour: 120 }),
  );
});

it("lists connected bots with their agent, relinks one, and connects another to a chosen agent", async () => {
  const user = renderButton();
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  const dialog = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const connected = await within(dialog).findByRole("list", { name: "Connected channel bots" });
  expect(connected).toHaveTextContent("Home bot");
  expect(connected).toHaveTextContent("Telegram");
  expect(connected).toHaveTextContent("Owner linked");
  // A bot without an agent reaches NyxBot.
  const agentFor = within(connected).getByRole("combobox", { name: "Agent for Home bot" });
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
  const available = within(dialog).getByRole("list", { name: "Channel bots you can connect" });
  expect(available).not.toHaveTextContent("Home bot");
  await user.click(within(available).getByRole("combobox", { name: "Agent for Dev Notifications" }));
  await user.click(screen.getByRole("option", { name: "writer" }));
  await user.click(within(available).getByRole("button", { name: "Connect Dev Notifications" }));
  const link = await within(dialog).findByRole("region", { name: "Owner link" });
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
  expect(writes.filter((write) => write.method === "POST").at(-1)?.body).toEqual({
    bot_id: "bot-dc",
    agent_id: "agent-writer",
  });
});

it("opens a bot's chats and saves who can talk and how it answers", async () => {
  const user = renderButton();
  await user.click(screen.getByRole("button", { name: "NyxBot settings" }));
  const dialog = await screen.findByRole("dialog", { name: "NyxBot settings" });
  const connected = await within(dialog).findByRole("list", { name: "Connected channel bots" });
  const toggle = within(connected).getByRole("button", { name: "Chats and who can talk" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  await user.click(toggle);
  const list = await within(connected).findByRole("list", { name: "Chats of Home bot" });
  // Groups have reply and member settings; private chats do not.
  expect(within(list).getByRole("combobox", { name: "Replies in Team chat" })).toHaveTextContent(
    "When mentioned",
  );
  expect(within(list).queryByRole("combobox", { name: "Replies in Alice" })).not.toBeInTheDocument();
  await user.click(within(list).getByRole("combobox", { name: "Replies in Team chat" }));
  await user.click(screen.getByRole("option", { name: "Every message" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/channels/channel-1/chats/chat-g",
      body: { reply_mode: "all" },
    }),
  );
  await user.click(
    within(list).getByRole("switch", { name: "Let the agent post in Team chat on its own" }),
  );
  await waitFor(() =>
    expect(writes).toContainEqual({
      method: "PATCH",
      endpoint: "/assistant/nyxagent/channels/channel-1/chats/chat-g",
      body: { allow_posts: true },
    }),
  );
  await user.click(
    within(connected).getByRole("combobox", {
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
