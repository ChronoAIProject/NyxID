import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nyxAgentLiveRefresh } from "./use-nyxagent-live";
import {
  NOTIFICATION_SOURCE_READ_DELAY_MS,
  notificationQueryKeys,
  sourceHasUnread,
  unreadSources,
  useMarkNotificationsRead,
  useNotificationList,
  useNotificationSourceRead,
  useNotificationSummary,
} from "./use-assistant-notifications";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { NotificationSummary } from "@/schemas/assistant-notifications";
import { useAuthStore } from "@/stores/auth-store";

const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });

function row(id: string, extra: Record<string, unknown> = {}) {
  return {
    id, kind: "agent_update", count: 1,
    source: { type: "conversation", id: "nyxa-1", label: "Research" },
    title: `Row ${id}`, preview: "", url: "https://nyxid.example/assistant?c=nyxa-1",
    read: false, resolved: false,
    created_at: "2026-11-10T08:00:00Z", updated_at: "2026-11-10T08:05:00Z",
    ...extra,
  };
}

const requests: { endpoint: string; method: string; body?: unknown }[] = [];
let summary: Record<string, unknown>;
/** The fake server remembers what was marked read, as the real one does. */
const readIds = new Set<string>();

beforeEach(() => {
  requests.length = 0;
  readIds.clear();
  summary = { unread_count: 1, needs_decision_count: 0, by_source: [{ source_type: "conversation", id: "nyxa-1", unread: 1 }] };
  useAuthStore.getState().setUser({
    id: "owner", email: "owner@example.com", display_name: "Owner", avatar_url: null,
    email_verified: true, mfa_enabled: false, is_admin: false, is_active: true,
    created_at: "2026-11-06T00:00:00Z",
    capabilities: { enabled_features: [FEATURE_FLAG.NYXAGENT_ENGINE, FEATURE_FLAG.ASSISTANT_NOTIFICATIONS] },
  });
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const method = init.method ?? "GET";
    const body = typeof init.body === "string" ? JSON.parse(init.body) as unknown : undefined;
    requests.push({ endpoint, method, body });
    if (endpoint === "/assistant/nyxagent/notifications/summary") return json(summary);
    if (endpoint === "/assistant/nyxagent/notifications/read") {
      if (body && typeof body === "object" && "ids" in body) for (const id of (body as { ids: string[] }).ids) readIds.add(id);
      return new Response(null, { status: 204 });
    }
    if (endpoint.startsWith("/assistant/nyxagent/notifications?")) {
      const query = new URLSearchParams(endpoint.slice(endpoint.indexOf("?") + 1));
      const cursor = query.get("cursor");
      const stored = (id: string, extra: Record<string, unknown> = {}) => row(id, { read: readIds.has(id), ...extra });
      return json(cursor
        ? { items: [stored("n-3")], next_cursor: null, as_of: "2026-11-10T08:10:00Z" }
        : { items: [stored("n-1"), stored("n-2", { read: true })], next_cursor: "page-2", as_of: "2026-11-10T08:10:00Z" });
    }
    throw new Error(`Unexpected request: ${method} ${endpoint}`);
  };
});

afterEach(() => {
  client.clear();
  vi.restoreAllMocks();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: null });
  Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
});

describe("useNotificationList", () => {
  it("pages newest first by cursor and sends the chip's kind filter", async () => {
    const { result } = renderHook(() => useNotificationList("chats"), { wrapper });
    await waitFor(() => expect(result.current.data?.pages).toHaveLength(1));
    const first = requests.find((request) => request.endpoint.startsWith("/assistant/nyxagent/notifications?"));
    const query = new URLSearchParams(first!.endpoint.split("?")[1]);
    expect(query.get("limit")).toBe("30");
    expect(query.get("kind")).toBe("chat_update,agent_update,group_mention");
    expect(query.get("cursor")).toBeNull();
    expect(result.current.hasNextPage).toBe(true);
    await act(async () => { await result.current.fetchNextPage(); });
    await waitFor(() => expect(result.current.data?.pages).toHaveLength(2));
    const second = requests.filter((request) => request.endpoint.includes("cursor=page-2"));
    expect(second).toHaveLength(1);
    expect(result.current.data?.pages.flatMap((page) => page.items.map((item) => item.id))).toEqual(["n-1", "n-2", "n-3"]);
    expect(result.current.data?.pages[0]?.as_of).toBe("2026-11-10T08:10:00Z");
  });

  it("sends no kind filter for All", async () => {
    const { result } = renderHook(() => useNotificationList("all"), { wrapper });
    await waitFor(() => expect(result.current.data?.pages).toHaveLength(1));
    const first = requests.find((request) => request.endpoint.startsWith("/assistant/nyxagent/notifications?"));
    expect(first!.endpoint).not.toContain("kind=");
    expect(first!.endpoint).not.toContain("pending_only");
  });

  it("asks for pending decisions only under Needs you, and nowhere else", async () => {
    const { result } = renderHook(() => useNotificationList("needs_you"), { wrapper });
    await waitFor(() => expect(result.current.data?.pages).toHaveLength(1));
    const first = requests.find((request) => request.endpoint.startsWith("/assistant/nyxagent/notifications?"));
    const query = new URLSearchParams(first!.endpoint.split("?")[1]);
    expect(query.get("kind")).toBe("needs_decision");
    expect(query.get("pending_only")).toBe("true");
    requests.length = 0;
    const chats = renderHook(() => useNotificationList("chats"), { wrapper });
    await waitFor(() => expect(chats.result.current.data?.pages).toHaveLength(1));
    const second = requests.find((request) => request.endpoint.startsWith("/assistant/nyxagent/notifications?"));
    expect(second!.endpoint).not.toContain("pending_only");
  });
});

describe("useMarkNotificationsRead", () => {
  it("turns the named rows read in every cached page and then re-reads the badge", async () => {
    const list = renderHook(() => useNotificationList("all"), { wrapper });
    const badge = renderHook(() => useNotificationSummary(), { wrapper });
    await waitFor(() => expect(list.result.current.data?.pages).toHaveLength(1));
    await waitFor(() => expect(badge.result.current.data?.unread_count).toBe(1));
    summary = { ...summary, unread_count: 0, by_source: [] };
    const mark = renderHook(() => useMarkNotificationsRead(), { wrapper });
    await act(async () => { await mark.result.current.mutateAsync({ ids: ["n-1"] }); });
    expect(requests.find((request) => request.endpoint.endsWith("/notifications/read"))?.body).toEqual({ ids: ["n-1"] });
    await waitFor(() => expect(list.result.current.data?.pages[0]?.items.map((item) => item.read)).toEqual([true, true]));
    await waitFor(() => expect(badge.result.current.data?.unread_count).toBe(0));
  });
});

describe("useNotificationSourceRead", () => {
  it("reports the open thread read after a pause, only while the tab is visible and only when it has unread rows", async () => {
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    const { rerender } = renderHook(
      ({ conversationId }: { conversationId?: string }) =>
        useNotificationSourceRead({ conversationId, enabled: true }),
      { wrapper, initialProps: { conversationId: "nyxa-1" } },
    );
    await waitFor(() => expect(requests.some((request) => request.endpoint.endsWith("/summary"))).toBe(true));
    await new Promise((resolve) => setTimeout(resolve, NOTIFICATION_SOURCE_READ_DELAY_MS + 100));
    expect(requests.filter((request) => request.endpoint.endsWith("/notifications/read"))).toHaveLength(0);

    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    act(() => { document.dispatchEvent(new Event("visibilitychange")); });
    await waitFor(
      () => expect(requests.filter((request) => request.endpoint.endsWith("/notifications/read"))).toHaveLength(1),
      { timeout: NOTIFICATION_SOURCE_READ_DELAY_MS + 1000 },
    );
    expect(requests.find((request) => request.endpoint.endsWith("/notifications/read"))?.body).toEqual({
      source: { type: "conversation", id: "nyxa-1" },
    });

    // A thread the summary does not list is never reported.
    summary = { unread_count: 1, needs_decision_count: 0, by_source: [{ source_type: "conversation", id: "nyxa-other", unread: 1 }] };
    await act(async () => { await client.invalidateQueries({ queryKey: notificationQueryKeys.root("owner") }); });
    rerender({ conversationId: "nyxa-2" });
    await new Promise((resolve) => setTimeout(resolve, NOTIFICATION_SOURCE_READ_DELAY_MS + 100));
    expect(requests.filter((request) => request.endpoint.endsWith("/notifications/read"))).toHaveLength(1);
  });
});

describe("summary helpers", () => {
  const base: NotificationSummary = {
    unread_count: 3,
    needs_decision_count: 1,
    by_source: [
      { source_type: "conversation", id: "nyxa-1", unread: 1 },
      { source_type: "channel_chat", id: "thread-1", unread: 1, conversation_id: "nyxa-2" },
      { source_type: "collaboration_group", id: "nyxg-1", unread: 1 },
      { source_type: "group", id: "nyxg-2", unread: 0 },
    ],
  };

  it("maps sources onto sidebar threads and crews; a channel chat is its own thread", () => {
    const sources = unreadSources(base);
    expect([...sources.conversationIds]).toEqual(["nyxa-1", "nyxa-2", "thread-1"]);
    expect([...sources.groupIds]).toEqual(["nyxg-1"]);
    expect(unreadSources(undefined).conversationIds.size).toBe(0);
    // Newer servers send the thread id as the channel source id itself.
    expect([...unreadSources({ ...base, by_source: [{ source_type: "channel_chat", id: "nyxa-5", unread: 2 }] }).conversationIds]).toEqual(["nyxa-5"]);
  });

  it("decides whether the open source needs a read, trusting a truncated aggregate less", () => {
    expect(sourceHasUnread(base, { conversationId: "nyxa-2" })).toBe(true);
    expect(sourceHasUnread(base, { groupId: "nyxg-2" })).toBe(false);
    expect(sourceHasUnread(base, { conversationId: "nyxa-9" })).toBe(false);
    expect(sourceHasUnread({ ...base, by_source_truncated: true }, { conversationId: "nyxa-9" })).toBe(true);
    expect(sourceHasUnread({ ...base, unread_count: 0, by_source_truncated: true }, { conversationId: "nyxa-9" })).toBe(false);
  });
});

describe("live events", () => {
  it("refreshes the feed and the badge at once on a notification frame", () => {
    const refresh = nyxAgentLiveRefresh("owner", { type: "notification", id: "n-1" }, new Map());
    expect(refresh.now).toEqual([notificationQueryKeys.root("owner")]);
    expect(refresh.lists).toEqual([]);
  });
});
