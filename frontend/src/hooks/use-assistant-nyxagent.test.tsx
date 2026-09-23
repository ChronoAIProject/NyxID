import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { continuationForAll, continuationText, useNyxAgentAssistantChat } from "./use-assistant-nyxagent";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { useAuthStore } from "@/stores/auth-store";
import type { NyxAgentHistory } from "@/schemas/assistant-nyxagent";

const id = `nyxa-${"a".repeat(32)}`;
const json = (value: unknown) => new Response(JSON.stringify(value));
let page: NyxAgentHistory;
let requests: string[];
let client: QueryClient;

function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

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
    created_at: "2026-09-17T00:00:00Z",
  });
  nyxAgentTransport.clear();
  requests = [];
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  page = {
    conversation: {
      id,
      title: "Question",
      model: "nyxagent/chat",
    access_mode: "ask",
      created_at: "2026-09-17T00:00:00Z",
      last_message_at: "2026-09-17T00:00:00Z",
      message_count: 1,
      pending_acknowledgements: 0,
      active_turn: { turn_id: "turn", started_at: "2026-09-17T00:00:00Z", activities: [], attachments: [] },
      context_reset_at: null,
    },
    messages: [{
      id: "user",
      turn_id: "turn",
      seq: 1,
      role: "user",
      text: "Question",
      status: "completed",
      error_code: null,
      created_at: "2026-09-17T00:00:00Z",
      activities: [], attachments: [],
    }],
    before_seq: null,
    acknowledgements: [],
    approvals: [],
  };
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    requests.push(`${init.method} ${endpoint}`);
    if (endpoint.endsWith("/models")) return json([{ id: "nyxagent/chat", label: "chat" }]);
    if (endpoint.includes(`/conversations/${id}`)) {
      if (init.method === "PATCH") {
        page.conversation.title = "Renamed";
        return json(page.conversation);
      }
      if (init.method === "DELETE") return new Response(null, { status: 204 });
      return json(page);
    }
    return json({ conversations: [page.conversation], next_cursor: null });
  };
});

afterEach(() => {
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.getState().setUser(null);
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("polls only selected history every two seconds and refreshes the index once on settlement",
  async () => {
    const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
      selectedConversationId: id,
      onConversationAdopted: vi.fn(),
    }), { wrapper });
    await waitFor(() => expect(result.current.isStreaming).toBe(true));
    const historyReads = () => requests.filter((r) => r.includes(`/conversations/${id}`)).length;
    const indexReads = () => requests.filter((r) => r.includes("/conversations?limit=")).length;
    const initialIndexReads = indexReads();
    // Multiple active polls must never refetch the (potentially paginated) index.
    await waitFor(() => expect(historyReads()).toBeGreaterThanOrEqual(3), { timeout: 5000 });
    expect(indexReads()).toBe(initialIndexReads);
    page.conversation.active_turn = null;
    page.messages.push({
      id: "assistant",
      turn_id: "turn",
      seq: 2,
      role: "assistant",
      text: "Durable answer",
      status: "completed",
      error_code: null,
      activities: [], attachments: [],
      created_at: "2026-09-17T00:00:01Z",
    });
    await waitFor(() => {
      expect(result.current.session.messages.at(-1)?.content).toBe("Durable answer");
      expect(indexReads()).toBe(initialIndexReads + 1);
    }, { timeout: 4000 });
    expect(result.current.isStreaming).toBe(false);
    expect(requests.some((r) => r.startsWith("POST"))).toBe(false);
    unmount();
  }, 10_000,
);

it("refreshes profiles after a first send provisions the assistant credential", async () => {
  page.conversation.active_turn = null;
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue();
  const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
    onConversationAdopted: vi.fn(),
  }), { wrapper });
  await waitFor(() => expect(requests.filter((r) => r.endsWith("/models"))).toHaveLength(1));
  await act(() => result.current.send("Question"));
  expect(send).toHaveBeenCalledOnce();
  expect(requests.filter((r) => r.endsWith("/models"))).toHaveLength(2);
  unmount();
});

it("exposes rename and deletion and clears visible data on identity transition", async () => {
  page.conversation.active_turn = null;
  const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: id,
    onConversationAdopted: vi.fn(),
  }), { wrapper });
  await waitFor(() => expect(result.current.session.title).toBe("Question"));
  await act(() => result.current.renameConversation(id, "Renamed"));
  expect(result.current.session.title).toBe("Renamed");
  await act(() => result.current.deleteConversation(id));
  expect(result.current.conversations).toEqual([]);
  act(() => useAuthStore.getState().setUser(null));
  expect(result.current.session.messages).toEqual([]);
  unmount();
});

it("does not fetch when the engine flag is disabled", () => {
  const { unmount } = renderHook(() => useNyxAgentAssistantChat({
    enabled: false,
    onConversationAdopted: vi.fn(),
  }), { wrapper });
  expect(requests).toEqual([]);
  unmount();
});

it("polls pending acknowledgements after settlement, throttles decisions, and resumes only on allow",
  async () => {
    page.conversation.active_turn = null;
    page.conversation.pending_acknowledgements = 1;
    page.acknowledgements = [{
      id: "12345678-1234-4123-8123-123456789012",
      kind: "account", status: "pending", summary: "Manage account",
      service_slug: null, service_name: null, tool_name: null,
      created_at: "2026-09-17T00:00:00Z", decided_at: null,
      expires_at: "2026-09-17T00:15:00Z",
    }];
    const { result } = renderHook(() => useNyxAgentAssistantChat({
      selectedConversationId: id, onConversationAdopted: vi.fn(),
    }), { wrapper });
    await waitFor(() => expect(result.current.acknowledgements).toHaveLength(1));
    const historyReads = () => requests.filter((r) => r.includes(`/conversations/${id}`)).length;
    await waitFor(() => expect(historyReads()).toBeGreaterThanOrEqual(2), { timeout: 3500 });
    let now = Date.now();
    vi.spyOn(Date, "now").mockImplementation(() => now);
    const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
    const decide = vi.spyOn(nyxAgentTransport, "decide").mockImplementation(async (_c, _i, choice) => {
      page.acknowledgements[0]!.status = choice === "allow" ? "allowed" : "denied";
      page.conversation.pending_acknowledgements = 0;
      return page.acknowledgements[0]!;
    });
    const deny = { id: page.acknowledgements[0]!.id, choice: "deny" as const };
    await act(() => result.current.decideAcknowledgement(deny));
    await act(async () => {
      await expect(result.current.decideAcknowledgement(deny)).rejects.toThrow("wait a moment");
    });
    expect(decide).toHaveBeenCalledOnce();
    expect(send).not.toHaveBeenCalled();
    now += 750;
    const allow = { ...deny, choice: "allow" as const };
    await act(() => result.current.decideAcknowledgement(allow));
    expect(decide).toHaveBeenCalledTimes(2);
    // Allow resumes the assistant with a visible continuation turn.
    expect(send).toHaveBeenCalledExactlyOnceWith(
      id,
      "Approved: account management for this chat. Continue.",
      expect.any(Function),
    );
    expect(requests.some((r) => r.startsWith("POST"))).toBe(false);
  }, 8000,
);

it("sends one continuation after a turn settles for cards allowed while it ran, never after Stop",
  async () => {
    page.conversation.pending_acknowledgements = 2;
    const card = (suffix: string, kind: "service" | "account") => ({
      id: `12345678-1234-4123-8123-12345678901${suffix}`,
      kind, status: "pending" as const, summary: "Card",
      service_slug: kind === "service" ? "github" : null,
      service_name: kind === "service" ? "GitHub" : null,
      tool_name: null,
      created_at: "2026-09-17T00:00:00Z", decided_at: null,
      expires_at: "2026-09-17T00:15:00Z",
    });
    page.acknowledgements = [card("1", "service"), card("2", "account")];
    const { result, rerender } = renderHook(() => useNyxAgentAssistantChat({
      selectedConversationId: id, onConversationAdopted: vi.fn(),
    }), { wrapper });
    await waitFor(() => expect(result.current.acknowledgements).toHaveLength(2));
    let now = Date.now();
    vi.spyOn(Date, "now").mockImplementation(() => now);
    const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
    vi.spyOn(nyxAgentTransport, "decide").mockImplementation(async (_c, ackId) => {
      const row = page.acknowledgements.find((ack) => ack.id === ackId)!;
      row.status = "allowed";
      return row;
    });
    let running = true;
    vi.spyOn(nyxAgentTransport, "isRunning").mockImplementation(() => running);
    // The user allows both cards before the assistant finishes its reply.
    for (const ack of page.acknowledgements) {
      await act(() => result.current.decideAcknowledgement({ id: ack.id, choice: "allow" }));
      now += 750;
    }
    rerender();
    expect(send).not.toHaveBeenCalled();
    // The turn settles: exactly one continuation carries both approvals.
    running = false;
    rerender();
    await waitFor(() => expect(send).toHaveBeenCalledOnce());
    expect(send).toHaveBeenCalledWith(
      id,
      "Approved: this chat may use GitHub. Continue.\n" +
        "Approved: account management for this chat. Continue.",
      expect.any(Function),
    );
    rerender();
    expect(send).toHaveBeenCalledOnce();

    // A turn the user stopped is not resumed.
    running = true;
    page.acknowledgements = [card("3", "service")];
    await act(async () => {
      await result.current.decideAcknowledgement({ id: page.acknowledgements[0]!.id, choice: "allow" });
    });
    const history = nyxAgentTransport.getHistory(id)!;
    vi.spyOn(nyxAgentTransport, "getHistory").mockReturnValue({
      ...history,
      messages: [
        ...history.messages,
        {
          id: "stopped", turn_id: "turn", seq: 2, role: "assistant", text: "",
          status: "failed", error_code: "cancelled",
          created_at: "2026-09-17T00:00:01Z", activities: [], attachments: [],
        },
      ],
    });
    running = false;
    rerender();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(send).toHaveBeenCalledOnce();
  }, 8000,
);

it("phrases continuation turns per acknowledgement kind", () => {
  const base = {
    id: "12345678-1234-4123-8123-123456789012",
    status: "allowed" as const,
    summary: "Delete agent key ci-bot",
    service_slug: null,
    service_name: null,
    tool_name: null,
    created_at: "2026-09-17T00:00:00Z",
    decided_at: "2026-09-17T00:00:01Z",
    expires_at: "2026-09-17T00:15:00Z",
  };
  expect(
    continuationText({ ...base, kind: "service", service_slug: "github", service_name: "GitHub" }),
  ).toBe("Approved: this chat may use GitHub. Continue.");
  expect(continuationText({ ...base, kind: "service", service_slug: "github" })).toBe(
    "Approved: this chat may use github. Continue.",
  );
  expect(continuationText({ ...base, kind: "account" })).toBe(
    "Approved: account management for this chat. Continue.",
  );
  expect(continuationForAll([{ ...base, kind: "account" }])).toBe(
    "Approved: account management for this chat. Continue.",
  );
  expect(continuationText({ ...base, kind: "action", tool_name: "delete_agent_key" })).toBe(
    `Confirmed: Delete agent key ci-bot (acknowledgement_id ${base.id}). Retry it now.`,
  );
});
