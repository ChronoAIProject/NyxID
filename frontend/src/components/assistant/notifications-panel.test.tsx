import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { notificationQueryKeys } from "@/hooks/use-assistant-notifications";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { NotificationsPanel } from "./notifications-panel";

const mocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  update: vi.fn(),
  current: { panelTab: undefined as string | undefined },
  toastError: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => mocks.navigate }));
vi.mock("@/hooks/use-account-panel", () => ({
  useAccountPanel: () => ({ current: { panel: "notifications", ...mocks.current }, update: mocks.update }),
}));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError } }));

const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });

function row(id: string, extra: Record<string, unknown> = {}) {
  return {
    id, kind: "agent_update", count: 1,
    source: { type: "conversation", id: "nyxa-1", label: "Research", conversation_id: "nyxa-1" },
    title: `Row ${id}`, preview: `Preview of ${id}`, url: `${window.location.origin}/assistant?c=nyxa-1`,
    read: false, resolved: false,
    created_at: "2026-11-10T08:00:00Z", updated_at: new Date().toISOString(),
    ...extra,
  };
}

const requests: { endpoint: string; method: string; body?: unknown }[] = [];
let pages: Record<string, { items: Record<string, unknown>[]; next_cursor: string | null; as_of: string }>;
let listStatus = 200;
/** The fake server remembers what was marked read, as the real one does. */
const readIds = new Set<string>();

beforeEach(() => {
  requests.length = 0;
  readIds.clear();
  listStatus = 200;
  mocks.navigate.mockReset();
  mocks.update.mockReset();
  mocks.toastError.mockReset();
  mocks.current = { panelTab: undefined };
  pages = {
    first: {
      items: [
        row("n-1", { count: 3 }),
        row("n-2", { kind: "needs_decision", title: "Allow Notion write?", read: true }),
        row("n-3", { kind: "automation_problem", read: true, source: { type: "trigger", id: "trg-1", label: "Morning digest", agent_id: "agent-1" }, url: `${window.location.origin}/assistant/automations?agent=agent-1` }),
      ],
      next_cursor: "page-2",
      as_of: "2026-11-10T08:10:00Z",
    },
    "page-2": {
      items: [
        row("n-4", { kind: "chat_update", source: { type: "channel_chat", id: "thread-1", label: "Family group", conversation_id: "nyxa-7" }, url: "" }),
        row("n-5", { kind: "automation_result", title: "Partner report", url: "https://partner.example/report", source: { type: "trigger", id: "trg-2", label: "Partner digest" } }),
        row("n-6", { kind: "agent_update", title: "Scripted link", url: "javascript:alert(1)" }),
      ],
      next_cursor: null,
      as_of: "2026-11-10T08:10:00Z",
    },
  };
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
    if (endpoint === "/assistant/nyxagent/notifications/read") {
      if (body && typeof body === "object" && "ids" in body) for (const id of (body as { ids: string[] }).ids) readIds.add(id);
      return new Response(null, { status: 204 });
    }
    if (endpoint === "/assistant/nyxagent/notifications/summary") return json({ unread_count: 0, needs_decision_count: 0, by_source: [] });
    if (endpoint.startsWith("/assistant/nyxagent/notifications?")) {
      if (listStatus !== 200) return json({ error: "boom", message: "Server exploded", error_code: 5000 }, listStatus);
      const query = new URLSearchParams(endpoint.slice(endpoint.indexOf("?") + 1));
      const page = pages[query.get("cursor") ?? "first"]!;
      return json({ ...page, items: page.items.map((item) => (readIds.has(item.id as string) ? { ...item, read: true } : item)) });
    }
    throw new Error(`Unexpected request: ${method} ${endpoint}`);
  };
});

afterEach(() => {
  cleanup();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: null });
});

function renderPanel(onClose = vi.fn()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(<QueryClientProvider client={client}><NotificationsPanel onClose={onClose} opener={() => null} /></QueryClientProvider>);
  return { client, onClose };
}

const listRequests = () => requests.filter((request) => request.endpoint.startsWith("/assistant/nyxagent/notifications?"));
const readRequests = () => requests.filter((request) => request.endpoint.endsWith("/notifications/read"));

describe("NotificationsPanel", () => {
  it("lists rows newest first with unread styling, counts, decision badges and previews", async () => {
    renderPanel();
    const dialog = await screen.findByRole("dialog", { name: "Notifications" });
    const list = await within(dialog).findByRole("list", { name: "Notifications" });
    const rows = within(list).getAllByRole("button");
    expect(rows.map((item) => item.getAttribute("aria-label"))).toEqual([
      "Row n-1, unread",
      "Allow Notion write?, needs your decision",
      "Row n-3",
    ]);
    expect(rows[0]).toHaveAttribute("data-unread");
    expect(rows[0]).toHaveTextContent("3 new");
    expect(rows[0]).toHaveTextContent("Preview of n-1");
    expect(rows[1]).not.toHaveAttribute("data-unread");
    expect(rows[1]).toHaveTextContent("Needs you");
    expect(rows[2]).toHaveTextContent("Morning digest");
  });

  it("changes the filter in the URL (history replaced) and asks the server for that kind subset", async () => {
    const user = userEvent.setup();
    renderPanel();
    await screen.findByRole("list", { name: "Notifications" });
    const chips = screen.getByRole("tablist", { name: "Filter notifications" });
    expect(within(chips).getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["All", "Needs you", "Automations", "Chats"]);
    await user.click(within(chips).getByRole("tab", { name: "Needs you" }));
    expect(mocks.update).toHaveBeenCalledWith({ panelTab: "needs_you" }, "replace");
    cleanup();
    mocks.current = { panelTab: "automations" };
    renderPanel();
    await waitFor(() => expect(listRequests().some((request) => request.endpoint.includes("kind=automation_result%2Cautomation_problem"))).toBe(true));
    expect(within(screen.getByRole("tablist", { name: "Filter notifications" })).getByRole("tab", { name: "Automations" })).toHaveAttribute("aria-selected", "true");
    await user.click(screen.getByRole("tab", { name: "All" }));
    expect(mocks.update).toHaveBeenLastCalledWith({ panelTab: undefined }, "replace");
  });

  it("loads the next page on request and keeps the earlier rows", async () => {
    const user = userEvent.setup();
    renderPanel();
    await screen.findByRole("list", { name: "Notifications" });
    await user.click(screen.getByRole("button", { name: "Load more" }));
    await waitFor(() => expect(within(screen.getByRole("list", { name: "Notifications" })).getAllByRole("button")).toHaveLength(6));
    expect(listRequests().at(-1)?.endpoint).toContain("cursor=page-2");
    expect(screen.queryByRole("button", { name: "Load more" })).not.toBeInTheDocument();
  });

  it("marks everything read behind the first page's fence and announces it", async () => {
    const user = userEvent.setup();
    renderPanel();
    await screen.findByRole("list", { name: "Notifications" });
    await user.click(screen.getByRole("button", { name: "Mark all as read" }));
    await waitFor(() => expect(readRequests()).toHaveLength(1));
    expect(readRequests()[0]?.body).toEqual({ all: true, before: "2026-11-10T08:10:00Z" });
    expect(await screen.findByRole("status")).toHaveTextContent("All notifications marked as read.");
  });

  it("opens a row's thread in the app and reports only unread rows read", async () => {
    const user = userEvent.setup();
    renderPanel();
    const list = await screen.findByRole("list", { name: "Notifications" });
    await user.click(within(list).getByRole("button", { name: "Row n-1, unread" }));
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/assistant", search: { c: "nyxa-1" } });
    await waitFor(() => expect(readRequests()).toHaveLength(1));
    expect(readRequests()[0]?.body).toEqual({ ids: ["n-1"] });
    expect(within(list).getByRole("button", { name: "Row n-1" })).not.toHaveAttribute("data-unread");
    await user.click(within(list).getByRole("button", { name: "Row n-3" }));
    expect(mocks.navigate).toHaveBeenLastCalledWith({ to: "/assistant/automations", search: { agent: "agent-1" } });
    expect(readRequests()).toHaveLength(1);
  });

  it("opens a foreign link in a new page that cannot reach the app, and never runs a scripted one", async () => {
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    const user = userEvent.setup();
    renderPanel();
    await screen.findByRole("list", { name: "Notifications" });
    await user.click(screen.getByRole("button", { name: "Load more" }));
    await user.click(await screen.findByRole("button", { name: "Partner report, unread" }));
    expect(open).toHaveBeenCalledWith("https://partner.example/report", "_blank", "noopener,noreferrer");
    expect(mocks.navigate).not.toHaveBeenCalled();
    await waitFor(() => expect(readRequests().map((request) => request.body)).toEqual([{ ids: ["n-5"] }]));
    // A `javascript:` link is refused; the row still leads to its own source in the app.
    await user.click(screen.getByRole("button", { name: "Scripted link, unread" }));
    expect(open).toHaveBeenCalledTimes(1);
    expect(mocks.navigate).toHaveBeenLastCalledWith({ to: "/assistant", search: { c: "nyxa-1" } });
    open.mockRestore();
  });

  it("follows a channel chat row to its thread when the server sent no link", async () => {
    const user = userEvent.setup();
    renderPanel();
    await screen.findByRole("list", { name: "Notifications" });
    await user.click(screen.getByRole("button", { name: "Load more" }));
    await user.click(await screen.findByRole("button", { name: "Row n-4, unread" }));
    expect(mocks.navigate).toHaveBeenLastCalledWith({ to: "/assistant", search: { c: "nyxa-7" } });
  });

  it("shows a retryable error and an empty state", async () => {
    const user = userEvent.setup();
    listStatus = 500;
    renderPanel();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Could not load notifications.");
    listStatus = 200;
    pages = { first: { items: [], next_cursor: null, as_of: "2026-11-10T08:10:00Z" } };
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Nothing new")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Mark all as read" })).toBeDisabled();
  });

  it("stops fetching and disappears when the Notifications feature turns off while open", async () => {
    const { client } = renderPanel();
    await screen.findByRole("list", { name: "Notifications" });
    const before = listRequests().length;
    act(() => {
      useAuthStore.getState().setUser({
        id: "owner", email: "owner@example.com", display_name: "Owner", avatar_url: null,
        email_verified: true, mfa_enabled: false, is_admin: false, is_active: true,
        created_at: "2026-11-06T00:00:00Z",
        capabilities: { enabled_features: [FEATURE_FLAG.NYXAGENT_ENGINE] },
      });
    });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Notifications" })).not.toBeInTheDocument());
    await act(async () => {
      await client.invalidateQueries({ queryKey: notificationQueryKeys.root("owner") });
    });
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(listRequests()).toHaveLength(before);
  });

  it("closes through the sheet and reports mark-read failures", async () => {
    const user = userEvent.setup();
    const { onClose } = renderPanel();
    await screen.findByRole("list", { name: "Notifications" });
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint.endsWith("/notifications/read")) return json({ error: "boom", message: "No", error_code: 5000 }, 500);
      return json({ items: [], next_cursor: null, as_of: "x" });
    };
    await user.click(screen.getByRole("button", { name: "Mark all as read" }));
    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledWith("Could not mark notifications as read"));
    await act(async () => { await user.keyboard("{Escape}"); });
    expect(onClose).toHaveBeenCalled();
  });
});
