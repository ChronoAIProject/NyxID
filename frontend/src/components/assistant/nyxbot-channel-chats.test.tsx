import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import {
  assistantAgentSchema,
  nyxAgentChannelAgentSchema,
} from "@/schemas/assistant-nyxagent";
import { GroupedList } from "@/components/ui/grouped-list";
import { ChannelChats } from "./nyxbot-channel-chats";

const at = "2026-11-04T00:00:00Z";
const agent = assistantAgentSchema.parse({
  id: "agent",
  name: "Researcher",
  kind: "specialist",
  status: "idle",
  created_at: at,
  last_active_at: at,
});
const channel = nyxAgentChannelAgentSchema.parse({
  id: "bot",
  channel_bot_id: "telegram",
  platform: "telegram",
  bot_label: "Helper",
  transport: "webhook",
  status: "active",
  owner_linked: true,
  agent_id: "agent",
  private_chats: "owner",
  created_at: at,
});
let client: QueryClient;
let reads: string[];
let writes: { endpoint: string; body: unknown }[];
let fail: boolean;
const chat = {
  id: "room",
  channel_agent_id: "bot",
  platform: "telegram",
  bot_label: "Helper",
  kind: "group",
  title: "Research room",
  agent_id: null,
  reply_mode: "mention",
  members: "owner",
  members_setting: "owner",
  owner_seen: true,
  allow_posts: false,
  last_message_at: at,
};

beforeEach(() => {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: at,
  });
  reads = [];
  writes = [];
  fail = false;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (init.method === "PATCH") {
      const body = JSON.parse(String(init.body));
      writes.push({ endpoint, body });
      if (fail)
        return new Response(
          JSON.stringify({ message: "Could not change this chat." }),
          { status: 409 },
        );
      return new Response(
        JSON.stringify({ chat: { ...chat, ...body }, note: "Chat updated." }),
      );
    }
    reads.push(endpoint);
    if (endpoint.includes("guest-readiness"))
      return new Response(JSON.stringify({ status: "ready" }));
    if (endpoint.endsWith("/chats"))
      return new Response(JSON.stringify({ chats: [chat] }));
    return new Response(JSON.stringify({ message: "Unexpected read" }), {
      status: 404,
    });
  };
});
afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
});
function mount() {
  render(
    <QueryClientProvider client={client}>
      <GroupedList title="Connected bot">
        <ChannelChats
          channel={channel}
          agents={[agent]}
          botAgentName="Researcher"
        />
      </GroupedList>
    </QueryClientProvider>,
  );
}

it("loads chats in a pushed page, groups readiness in footers and restores the disclosure focus", async () => {
  const user = userEvent.setup();
  mount();
  expect(reads).toEqual([]);
  const disclosure = screen.getByRole("button", {
    name: "Chats and who can talk",
  });
  await user.click(disclosure);
  expect(
    await screen.findByRole("heading", { name: /Research room · Group/ }),
  ).toBeVisible();
  expect(screen.getByRole("heading", { name: "Helper · Chats" })).toHaveFocus();
  for (const text of screen.getAllByText("Model ready for visitors")) {
    expect(text.closest("[data-grouped-footer]")).toBeInTheDocument();
    expect(text.closest("[data-grouped-list]")).toBeNull();
  }
  await user.click(screen.getByRole("button", { name: "Back to Channels" }));
  expect(disclosure).toHaveFocus();
  expect(writes).toEqual([]);
});

it("preserves private access, bulk posting and per-chat settings payloads", async () => {
  const user = userEvent.setup();
  mount();
  await user.click(
    screen.getByRole("button", { name: "Chats and who can talk" }),
  );
  await user.click(
    await screen.findByRole("combobox", {
      name: "Who can talk to Helper in private chats",
    }),
  );
  await user.click(screen.getByRole("option", { name: "Anyone" }));
  await waitFor(() =>
    expect(writes).toContainEqual({
      endpoint: "/assistant/nyxagent/channels/bot",
      body: { private_chats: "everyone" },
    }),
  );
  await user.click(
    screen.getByRole("checkbox", {
      name: "Also apply to existing private chats",
    }),
  );
  await user.click(
    screen.getByRole("button", { name: "Apply current setting" }),
  );
  await waitFor(() =>
    expect(writes).toContainEqual({
      endpoint: "/assistant/nyxagent/channels/bot",
      body: { private_chat_posts: false, apply_to_existing: true },
    }),
  );
  expect(
    screen.getByRole("checkbox", {
      name: "Also apply to existing private chats",
    }),
  ).not.toBeChecked();
  const room = screen.getByRole("region", { name: /Research room · Group/ });
  await user.click(
    within(room).getByRole("switch", {
      name: "Let the agent post in Research room on its own",
    }),
  );
  await waitFor(() =>
    expect(writes).toContainEqual({
      endpoint: "/assistant/nyxagent/channels/bot/chats/room",
      body: { allow_posts: true },
    }),
  );
  expect(await screen.findByText("Chat updated.")).toBeVisible();
});

it("keeps a per-chat failure visible in the active page's footer", async () => {
  const user = userEvent.setup();
  fail = true;
  mount();
  await user.click(
    screen.getByRole("button", { name: "Chats and who can talk" }),
  );
  await user.click(
    await screen.findByRole("switch", {
      name: "Let the agent post in Research room on its own",
    }),
  );
  const error = await screen.findByRole("alert");
  expect(error).toHaveTextContent("Could not change this chat.");
  expect(error.closest("[data-grouped-footer]")).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Helper · Chats" })).toBeVisible();
});
