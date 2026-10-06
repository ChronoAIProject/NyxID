import type { ComponentProps } from "react";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { nyxAgentChannelChatSchema } from "@/schemas/assistant-nyxagent";
import { parseNyxAgentLiveEvent } from "@/lib/assistant/nyxagent-live";
import { nyxAgentLiveRefresh } from "@/hooks/use-nyxagent-live";
import { ChannelThreads } from "./nyxbot-channel-threads";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, ...props }: ComponentProps<"a"> & { to: string; search?: Record<string, string> }) =>
    <a href={to + (search ? `?${new URLSearchParams(search)}` : "")} {...props} />,
}));
const at = "2026-10-04T00:00:00Z";
const chat = nyxAgentChannelChatSchema.parse({ id: "parent", channel_agent_id: "bot", platform: "telegram", bot_label: "Helper", kind: "group", reply_mode: "all", members: "everyone", conversation_id: "legacy", followed_thread_count: 1 });
const thread = { id: "child", parent_chat_id: "parent", conversation_id: "branch", agent_id: "agent", label: "Topic · Today", state: "active", kind: "topic", followed_at: at, last_admitted_at: at, expires_at: at, context_status: "metadata_only", context_message_count: 2, follow_readiness: "ready" };
let client: QueryClient;
let requests: string[];
let stopped: boolean;
let failStop: boolean;
beforeEach(() => {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useAuthStore.getState().setUser({ id: "owner", email: "owner@example.com", display_name: "Owner", avatar_url: null, email_verified: true, mfa_enabled: false, is_admin: false, is_active: true, created_at: at });
  requests = []; stopped = false; failStop = false;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    requests.push(endpoint);
    if (init.method === "POST") {
      if (failStop) return new Response(JSON.stringify({ message: "Please retry." }), { status: 503 });
      stopped = true;
      return new Response(JSON.stringify({ thread: { ...thread, state: "stopped" } }));
    }
    const history = endpoint.includes("state=all");
    const second = endpoint.includes("cursor=next");
    return new Response(JSON.stringify({ threads: stopped && !history ? [] : [{ ...thread, id: second ? "older" : "child", state: stopped ? "stopped" : "active" }], next_cursor: second || stopped ? null : "next" }));
  };
});
afterEach(() => { cleanup(); client.clear(); globalThis.__nyxidAssistantHttpMock = undefined; });
function mount() { render(<QueryClientProvider client={client}><ChannelThreads chat={chat} /></QueryClientProvider>); }

it("loads on expansion, pages history independently, and retains the transcript after stopping", async () => {
  const user = userEvent.setup(); mount();
  expect(requests).toEqual([]);
  expect(screen.getByRole("link", { name: "Chat history" })).toHaveAttribute("href", "/assistant?c=legacy");
  await user.click(screen.getByRole("button", { name: "Followed threads (1)" }));
  expect(await screen.findByRole("link", { name: "Topic · Today" })).toHaveAttribute("href", "/assistant?c=branch");
  expect(screen.getByText(/Every message still allows replies/)).toBeInTheDocument();
  expect(screen.getByText(/Follows the whole topic/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Show more threads" }));
  await waitFor(() => expect(requests.some((r) => r.includes("cursor=next"))).toBe(true));
  await user.click(screen.getAllByRole("button", { name: "Stop following" })[0]!);
  await screen.findByText("No active threads.");
  expect(requests).toContain("/assistant/nyxagent/channels/bot/chats/parent/threads/child/stop");
  await user.click(screen.getByRole("checkbox", { name: "Include stopped and expired threads" }));
  expect(await screen.findByText("Stopped")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Topic · Today" })).toHaveAttribute("href", "/assistant?c=branch");
});

it("shows stop failure without removing the active follow", async () => {
  failStop = true; const user = userEvent.setup(); mount();
  await user.click(screen.getByRole("button", { name: "Followed threads (1)" }));
  await user.click(await screen.findByRole("button", { name: "Stop following" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Please retry.");
  expect(screen.getByText("Following")).toBeInTheDocument();
});

it("accepts old parent responses and coalesces identifier-only thread changes", () => {
  expect(chat.threads).toBeUndefined();
  const event = parseNyxAgentLiveEvent(JSON.stringify({ type: "channel_thread", id: "child", channel_id: "bot", parent_id: "parent", conversation_id: "branch", text: "must be ignored" }));
  expect(event).not.toHaveProperty("text");
  expect(nyxAgentLiveRefresh("owner", event!, new Map())).toEqual({ now: [], lists: [["assistant", "nyxagent", "owner", "channels", "bot", "chats"], ["assistant", "nyxagent", "owner", "threads"]] });
});
