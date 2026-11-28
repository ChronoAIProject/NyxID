import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { continuationForAll, continuationText, useNyxAgentAssistantChat, useNyxAgentThreads } from "./use-assistant-nyxagent";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { nyxAgentDecisions } from "@/lib/assistant/nyxagent-decisions";
import { useAuthStore } from "@/stores/auth-store";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";
import type { NyxAgentAcknowledgement, NyxAgentHistory } from "@/schemas/assistant-nyxagent";

const id = `nyxa-${"a".repeat(32)}`;
const NYXBOT = "agent-nyxbot";
const nyxbotRef = { id: NYXBOT, kind: "nyxbot" as const, name: "NyxBot", destroyed: false };
function agentRow(fields: Record<string, unknown>) {
  return {
    id: NYXBOT, kind: "nyxbot", name: "NyxBot", description: "", specialty: null,
    created_by: "user", status: "idle", services: [], account_read: true,
    pending_requests: [], last_reply: null, home_conversation_id: null, memory_count: 0,
    created_at: "2026-09-17T00:00:00Z", last_active_at: "2026-09-17T00:00:00Z",
    destroyed_at: null, pending_acknowledgements: 0, channels: [],
    ...fields,
  };
}
const limits = {
  skip_destructive_confirmation: false, max_live_subagents: 8,
  max_concurrent_subagent_turns: 3, max_live_subagents_limit: 32,
  max_concurrent_subagent_turns_limit: 8,
};
let agents: ReturnType<typeof agentRow>[];
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
    cma_operations: [],
    conversation: {
      id,
      title: "Question",
      model: "nyxagent/chat",
      role: "orchestrator",
      agent: nyxbotRef,
      pending_events: 0,
      channel: null,
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
    waiting: [],
  };
  agents = [agentRow({})];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    requests.push(`${init.method} ${endpoint}`);
    if (endpoint.endsWith("/models")) return json([{ id: "nyxagent/chat", label: "chat" }]);
    if (endpoint.startsWith("/assistant/nyxagent/agents")) return json({ agents, limits });
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

it("keeps polling a CMA job after the assistant turn has ended", async () => {
  page.conversation.active_turn = null;
  page.cma_operations = [{
    id: "watch-1", turn_id: "turn", kind: "cma_response", operation_id: "resp_agt_test_1",
    agent_id: "agt_test", idempotency_key: null, status: "in_progress", phase: null,
    state: "waiting", reason: null, monitor_name: "NyxBot", created_at: "2026-10-22T10:00:00Z", checked_at: null,
  }];
  const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: id, onConversationAdopted: vi.fn(), threadsAgentId: NYXBOT,
  }), { wrapper });
  await waitFor(() => expect(result.current.isStreaming).toBe(false));
  const reads = () => requests.filter((request) => request.includes(`/conversations/${id}`)).length;
  await waitFor(() => expect(reads()).toBeGreaterThanOrEqual(1));
  await waitFor(() => expect(reads()).toBeGreaterThanOrEqual(2), { timeout: 6500 });
  expect(requests.some((request) => request.startsWith("POST"))).toBe(false);
  unmount();
}, 8000);

it("polls pending approvals through fifteen seconds after the latest pending deadline", async () => {
  vi.useFakeTimers();
  const start = Date.parse("2026-09-17T00:01:00Z");
  vi.setSystemTime(start);
  page.conversation.active_turn = null;
  const approval = {
    id: "old", service_slug: "sheets", service_name: "Sheets", summary: "Append row",
    status: "pending" as const, approval_mode: "per_request" as const, agent_key_prefix: "",
    created_at: page.conversation.created_at, expires_at: new Date(start - 60_000).toISOString(),
  };
  page.approvals = [
    approval,
    { ...approval, id: "last-pending", expires_at: new Date(start - 13_000).toISOString() },
    { ...approval, id: "decided", status: "approved", expires_at: new Date(start + 60_000).toISOString() },
  ];
  const { unmount } = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: id, onConversationAdopted: vi.fn(),
  }), { wrapper });
  const advance = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
  const reads = () => requests.filter((request) => request.includes(`/conversations/${id}?`)).length;

  await advance(0);
  const initial = reads();
  expect(initial).toBeGreaterThan(0);
  await advance(2000);
  expect(reads()).toBe(initial + 1);
  await advance(2000);
  expect(reads()).toBe(initial + 2);
  await advance(6000);
  expect(reads()).toBe(initial + 2);
  unmount();
});

it("polls only selected history every two seconds and refreshes the agent's threads once on settlement",
  async () => {
    const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
      selectedConversationId: id,
      onConversationAdopted: vi.fn(),
      threadsAgentId: NYXBOT,
    }), { wrapper });
    await waitFor(() => expect(result.current.isStreaming).toBe(true));
    const historyReads = () => requests.filter((r) => r.includes(`/conversations/${id}`)).length;
    const indexReads = () =>
      requests.filter((r) => r.includes(`/conversations?limit=100&agent_id=${NYXBOT}`)).length;
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

it("routes models server-side: no profile discovery, and a send refreshes the agents", async () => {
  page.conversation.active_turn = null;
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue();
  const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
    onConversationAdopted: vi.fn(),
  }), { wrapper });
  const agentReads = () =>
    requests.filter((r) => r.includes("/assistant/nyxagent/agents")).length;
  await waitFor(() => expect(agentReads()).toBe(1));
  await act(() => result.current.send("Question"));
  expect(send).toHaveBeenCalledOnce();
  expect(agentReads()).toBe(2);
  // Model discovery is intentionally owned by the selector hook, not turn transport.
  expect(requests.some((r) => r.endsWith("/models"))).toBe(false);
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
      decider: "user", decided_by: null, reason: null,
      service_slug: null, service_name: null, tool_name: null,
      created_at: "2026-09-17T00:00:00Z", decided_at: null,
      expires_at: "2099-09-17T00:15:00Z",
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
      page.acknowledgements[0]!.decided_by = "user";
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
      "Approved: account management for this chat. Continue. Then continue the original task and tell the user when it is finished or when you need their input.",
      expect.any(Function),
      expect.any(Function),
      expect.objectContaining({ acknowledgementId: "12345678-1234-4123-8123-123456789012" }),
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
      decider: "user" as const, decided_by: null, reason: null,
      service_slug: kind === "service" ? "github" : null,
      service_name: kind === "service" ? "GitHub" : null,
      tool_name: null,
      created_at: "2026-09-17T00:00:00Z", decided_at: null,
      expires_at: "2099-09-17T00:15:00Z",
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
      row.decided_by = "user";
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
      "Approved: this chat may use GitHub. Continue. Then continue the original task and tell the user when it is finished or when you need their input.\n" +
        "Approved: account management for this chat. Continue. Then continue the original task and tell the user when it is finished or when you need their input.",
      expect.any(Function),
      expect.any(Function),
      expect.objectContaining({ acknowledgementId: "12345678-1234-4123-8123-123456789011" }),
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

it.each(["specialist", "automation", "voice", "invocation"])(
  "leaves %s confirmation continuation to the server",
  async (kind) => {
    page.conversation.active_turn = null;
    page.conversation.pending_acknowledgements = 1;
    page.acknowledgements = [
      {
        id: "12345678-1234-4123-8123-123456789012",
        kind: kind === "invocation" ? "action" : "service",
        status: "pending",
        summary: "Use GitHub",
        decider: kind === "specialist" ? "orchestrator" : "user",
        trigger_run_id: kind === "automation" ? "run-id" : null,
        continuation_owner: ["voice", "invocation"].includes(kind) ? "server" : null,
        continuation_receipt_id: ["voice", "invocation"].includes(kind) ? "12345678-1234-4123-8123-123456789013" : null,
        decided_by: null,
        reason: null,
        service_slug: "github",
        service_name: "GitHub",
        tool_name: kind === "invocation" ? "nyxid__delete_node" : null,
        created_at: "2026-09-17T00:00:00Z",
        decided_at: null,
        expires_at: "2099-09-17T00:15:00Z",
      },
    ];
    const { result, unmount } = renderHook(
      () =>
        useNyxAgentAssistantChat({
          selectedConversationId: id,
          onConversationAdopted: vi.fn(),
        }),
      { wrapper },
    );
    await waitFor(() =>
      expect(result.current.acknowledgements).toHaveLength(1),
    );
    const send = vi
      .spyOn(nyxAgentTransport, "send")
      .mockResolvedValue(undefined);
    vi.spyOn(nyxAgentTransport, "decide").mockImplementation(async () => {
      page.acknowledgements[0] = {
        ...page.acknowledgements[0]!,
        status: "allowed",
        decided_by: "user",
      };
      return page.acknowledgements[0];
    });
    const reads = () =>
      requests.filter((r) =>
        r.startsWith(`GET /assistant/nyxagent/conversations/${id}?`),
      ).length;
    const before = reads();
    await act(() =>
      result.current.decideAcknowledgement({
        id: page.acknowledgements[0]!.id,
        choice: "allow",
      }),
    );
    // The server resumes the subagent itself; the page only refreshes history.
    await waitFor(() => expect(reads()).toBeGreaterThan(before));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(send).not.toHaveBeenCalled();
    unmount();
  },
);

function decisionNavigationFixture(running: boolean, destinationStatus?: number, destinationRunning = false) {
  const second = `nyxa-${"b".repeat(32)}`;
  const third = `nyxa-${"c".repeat(32)}`;
  const activeTurn = page.conversation.active_turn;
  if (!running) page.conversation.active_turn = null;
  page.conversation.pending_acknowledgements = 1;
  const card = {
    id: "12345678-1234-4123-8123-123456789012",
    kind: "action" as const, status: "pending" as const, summary: "Update ledger",
    decider: "user" as const, decided_by: null, reason: null,
    service_slug: "sheets", service_name: "Sheets", tool_name: "sheets_update_values",
    created_at: "2026-09-17T00:00:00Z", decided_at: null,
    expires_at: "2099-09-17T00:15:00Z",
  };
  page.acknowledgements = [card];
  let resolveDecision!: (response: Response) => void;
  const response = new Promise<Response>((resolve) => { resolveDecision = resolve; });
  const mock = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (request.endpoint.endsWith(`/acknowledgements/${card.id}`)) {
      requests.push(`${request.init.method} ${request.endpoint}`);
      return response;
    }
    if ([second, third].some((id) => request.endpoint.includes(`/conversations/${id}`))) {
      requests.push(`${request.init.method} ${request.endpoint}`);
      if (destinationStatus) return new Response(null, { status: destinationStatus });
      return json({
        ...page, acknowledgements: [],
        conversation: { ...page.conversation, id: request.endpoint.includes(second) ? second : third, active_turn: destinationRunning ? activeTurn : null },
      });
    }
    return mock(request);
  };
  const adopted = vi.fn();
  const hook = renderHook(
    ({ selected, enabled }: { selected: string; enabled: boolean }) => useNyxAgentAssistantChat({
      selectedConversationId: selected, enabled, onConversationAdopted: adopted,
    }),
    { wrapper, initialProps: { selected: id, enabled: true } },
  );
  return {
    ...hook, second, third, card, adopted,
    resolve: (choice: "allow" | "deny" = "allow", fields: Partial<NyxAgentAcknowledgement> = {}) => {
      const row: NyxAgentAcknowledgement = {
        ...card, status: choice === "allow" ? "allowed" : "denied", decided_by: "user", ...fields,
      };
      page.acknowledgements = page.acknowledgements.map((ack) => ack.id === card.id ? row : ack);
      resolveDecision(json(row));
    },
  };
}

it.each([[false, false], [false, true], [true, false], [true, true]])("keeps an in-flight Allow bound to its source across navigation (source running: %s, destination running: %s)", async (running, destinationRunning) => {
  const f = decisionNavigationFixture(running, undefined, destinationRunning);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  const invalidate = vi.spyOn(client, "invalidateQueries");
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" }); });
  await waitFor(() => expect(requests.some((r) => r.endsWith(`/acknowledgements/${f.card.id}`))).toBe(true));
  f.rerender({ selected: f.second, enabled: true });
  await waitFor(() => expect(f.result.current.conversation?.id).toBe(f.second));
  invalidate.mockClear();
  await act(async () => { f.resolve(); await decision; });
  if (running) {
    expect(send).not.toHaveBeenCalled();
    page.conversation.active_turn = null;
  }
  await waitFor(() => expect(send).toHaveBeenCalledOnce(), { timeout: 3500 });
  expect(send).toHaveBeenCalledWith(
    id, continuationText({ ...f.card, status: "allowed" }), expect.any(Function), expect.any(Function),
    expect.objectContaining({ acknowledgementId: f.card.id }),
  );
  expect(invalidate).toHaveBeenCalledWith(
    { queryKey: nyxBotQueryKeys.history("owner", id) }, { cancelRefetch: false },
  );
  // Even a late transport callback cannot move the owner back to this thread.
  f.rerender({ selected: f.third, enabled: true });
  act(() => send.mock.calls[0]![2](id));
  expect(f.adopted).not.toHaveBeenCalled();
  f.rerender({ selected: f.third, enabled: true });
  expect(send).toHaveBeenCalledOnce();
  f.unmount();
});

it.each([false, true])("a restricted selection preserves the original Allow (queued: %s)", async (queued) => {
  const f = decisionNavigationFixture(queued);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" }); });
  await waitFor(() => expect(requests.some((r) => r.endsWith(`/acknowledgements/${f.card.id}`))).toBe(true));
  f.rerender({ selected: f.second, enabled: false });
  await act(async () => { f.resolve(); await decision; });
  if (queued) page.conversation.active_turn = null;
  await waitFor(() => expect(send).toHaveBeenCalledOnce(), { timeout: 3500 });
  expect(send.mock.calls[0]?.[0]).toBe(id);
  expect(f.adopted).not.toHaveBeenCalled();
  f.unmount();
});

it.each([false, true])("an unmounted chat preserves an in-flight Allow through settlement (queued: %s)", async (queued) => {
  const f = decisionNavigationFixture(queued);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" }); });
  await waitFor(() => expect(requests.some((r) => r.endsWith(`/acknowledgements/${f.card.id}`))).toBe(true));
  f.unmount();
  await act(async () => { f.resolve(); await decision; });
  page.conversation.active_turn = null;
  await waitFor(() => expect(send).toHaveBeenCalledOnce(), { timeout: 3500 });
  expect(send.mock.calls[0]?.[0]).toBe(id);
  expect(f.adopted).not.toHaveBeenCalled();
});

it.each(["expired", "invalid deadline", "denied", "all expired"])("settlement drops %s cards before selecting continuation text and marker", async (state) => {
  const f = decisionNavigationFixture(true);
  const secondCard = { ...f.card, id: "12345678-1234-4123-8123-123456789013", summary: "Second ledger action" };
  page.acknowledgements.push(secondCard);
  await act(() => nyxAgentTransport.history(id));
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(2));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  let now = Date.now();
  vi.spyOn(Date, "now").mockImplementation(() => now);
  const original = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (!request.endpoint.endsWith(`/acknowledgements/${secondCard.id}`)) return original(request);
    const allowed = { ...secondCard, status: "allowed" as const, decided_by: "user" as const };
    page.acknowledgements = page.acknowledgements.map((ack) => ack.id === secondCard.id ? allowed : ack);
    return json(allowed);
  };
  await act(async () => {
    const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
    f.resolve(); await decision;
  });
  now += 750;
  await act(() => f.result.current.decideAcknowledgement({ id: secondCard.id, choice: "allow" }));
  expect(send).not.toHaveBeenCalled();
  page.acknowledgements = [
    { ...f.card, status: state === "denied" ? "denied" : "expired", expires_at: state === "invalid deadline" ? "invalid" : "2000-01-01T00:00:00Z" },
    { ...secondCard, decided_by: "user", status: state === "all expired" ? "expired" : "allowed", expires_at: "2099-01-01T00:00:00Z" },
  ];
  page.conversation.active_turn = null;
  await act(() => nyxAgentTransport.history(id));
  if (state === "all expired") expect(send).not.toHaveBeenCalled();
  else {
    expect(send).toHaveBeenCalledOnce();
    expect(send.mock.calls[0]?.[1]).not.toContain(f.card.id);
    expect(send.mock.calls[0]?.[1]).toContain(secondCard.id);
    expect(send.mock.calls[0]?.[4]).toMatchObject({ acknowledgementId: secondCard.id });
  }
  f.unmount();
});

it.each(["past", "exact", "invalid", "denied", "valid"])("an idle Allow rechecks %s eligibility at response time before immediate send", async (eligibility) => {
  const f = decisionNavigationFixture(false);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  const now = Date.now();
  vi.spyOn(Date, "now").mockReturnValue(now);
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" }); });
  await waitFor(() => expect(requests.some((r) => r.endsWith(`/acknowledgements/${f.card.id}`))).toBe(true));
  let finishRefresh!: () => void;
  const refresh = new Promise<void>((resolve) => { finishRefresh = resolve; });
  vi.spyOn(client, "invalidateQueries").mockImplementation(() => refresh);
  await act(async () => {
    f.resolve("allow", {
      status: eligibility === "denied" ? "denied" : "allowed",
      expires_at: eligibility === "invalid" ? "invalid" : new Date(now + (eligibility === "valid" ? 1000 : eligibility === "past" ? -1 : 0)).toISOString(),
    });
    await Promise.resolve();
  });
  if (eligibility === "valid") {
    // Continuation starts before mutation settlement waits on history refresh.
    await waitFor(() => expect(send).toHaveBeenCalledOnce());
    expect(send.mock.calls[0]?.[4]).toMatchObject({ acknowledgementId: f.card.id });
  } else expect(send).not.toHaveBeenCalled();
  await act(async () => { finishRefresh(); await decision; });
  f.unmount();
});

it.each(["Stop", "identity", "refusal"])("an unmounted source retains its %s fence", async (outcome) => {
  const f = decisionNavigationFixture(true);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  await act(async () => {
    const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
    f.resolve(); await decision;
  });
  f.unmount();
  page.conversation.active_turn = null;
  if (outcome === "Stop") page.messages.push({ ...page.messages[0]!, id: "stop", seq: 2, role: "assistant", status: "failed", error_code: "cancelled" });
  if (outcome === "identity") act(() => useAuthStore.getState().setUser(null));
  if (outcome === "refusal") {
    const original = globalThis.__nyxidAssistantHttpMock!;
    globalThis.__nyxidAssistantHttpMock = (request) => request.endpoint.includes(`/conversations/${id}`)
      ? new Response(null, { status: 403 }) : original(request);
  }
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 2200)); });
  expect(send).not.toHaveBeenCalled();
});

it.each([false, true])("refuses a decision after an actor transition in mutation middleware (round trip: %s)", async (roundTrip) => {
  let release!: () => void;
  const middleware = new Promise<void>((resolve) => { release = resolve; });
  client.setDefaultOptions({ mutations: { onMutate: () => middleware } });
  const f = decisionNavigationFixture(false);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" }); });
  const rejected = expect(decision).rejects.toMatchObject({ name: "AbortError", message: "Identity changed" });
  const owner = useAuthStore.getState().user!;
  act(() => {
    useAuthStore.getState().setUser({ ...owner, id: "another-owner" });
    if (roundTrip) useAuthStore.getState().setUser(owner);
  });
  await act(async () => { release(); await rejected; });
  expect(requests.filter((r) => r.includes("/acknowledgements/"))).toEqual([]);
  expect(send).not.toHaveBeenCalled();
  f.unmount();
});

it.each([false, true])("a saved control cannot adopt a newer actor's authority (round trip: %s)", async (roundTrip) => {
  const f = decisionNavigationFixture(false);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const saved = f.result.current.decideAcknowledgement;
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  const owner = useAuthStore.getState().user!;
  act(() => {
    useAuthStore.getState().setUser({ ...owner, id: "another-owner" });
    if (roundTrip) useAuthStore.getState().setUser(owner);
  });
  await act(async () => {
    await expect(saved({ id: f.card.id, choice: "allow" })).rejects.toMatchObject({ name: "AbortError" });
  });
  expect(requests.some((r) => r.includes("/acknowledgements/"))).toBe(false);
  expect(send).not.toHaveBeenCalled();
  f.unmount();
});

it.each(["account", "service", "operations", "skills", "used action"])("retains an allowed %s continuation after its pending deadline", async (kind) => {
  const f = decisionNavigationFixture(true);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  await act(async () => {
    const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
    f.resolve("allow", { kind: kind === "used action" ? "action" : kind as "account" | "service" | "operations" | "skills" });
    await decision;
  });
  expect(send).not.toHaveBeenCalled();
  page.acknowledgements = page.acknowledgements.map((row) => ({ ...row, expires_at: "2000-01-01T00:00:00Z", status: kind === "used action" ? "used" : "allowed" }));
  page.conversation.active_turn = null;
  await act(() => nyxAgentTransport.history(id));
  expect(send).toHaveBeenCalledOnce();
  expect(send.mock.calls[0]?.[4]).toMatchObject({ acknowledgementId: f.card.id });
  f.unmount();
});

it.each([null, "future_decider", "access_mode", "orchestrator"])("does not send a marker without a positive user decision (%s)", async (decidedBy) => {
  const f = decisionNavigationFixture(true);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  await act(async () => {
    const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
    f.resolve(); await decision;
  });
  page.acknowledgements = page.acknowledgements.map((row) => ({ ...row, decided_by: decidedBy }));
  page.conversation.active_turn = null;
  await act(() => nyxAgentTransport.history(id));
  expect(send).not.toHaveBeenCalled();
  f.unmount();
});

it.each(["Deny", "server", "orchestrator", "trigger"])("navigation preserves %s continuation ownership", async (kind) => {
  const f = decisionNavigationFixture(false);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  const choice = kind === "Deny" ? "deny" : "allow";
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice }); });
  await waitFor(() => expect(requests.some((r) => r.endsWith(`/acknowledgements/${f.card.id}`))).toBe(true));
  f.rerender({ selected: f.second, enabled: true });
  await act(async () => {
    f.resolve(choice, {
      ...(kind === "server" ? { continuation_owner: "server" as const } : {}),
      ...(kind === "orchestrator" ? { decider: "orchestrator" as const } : {}),
      ...(kind === "trigger" ? { trigger_run_id: "run-id" } : {}),
    });
    await decision;
  });
  expect(send).not.toHaveBeenCalled();
  f.unmount();
});

it.each([403, 404, "Stop"])("observes a queued source's %s after navigation without a local stream", async (outcome) => {
  const f = decisionNavigationFixture(true);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  await act(async () => {
    const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
    f.resolve(); await decision;
  });
  f.rerender({ selected: f.second, enabled: true });
  await waitFor(() => expect(f.result.current.conversation?.id).toBe(f.second));
  const mock = globalThis.__nyxidAssistantHttpMock!;
  let sourceReads = 0;
  globalThis.__nyxidAssistantHttpMock = (r) => {
    if (r.endpoint.includes(`/conversations/${id}`)) {
      sourceReads++;
      if (typeof outcome === "number") return new Response(null, { status: outcome });
    }
    return mock(r);
  };
  page.messages.push({ ...page.messages[0]!, id: "stopped", seq: 2, role: "assistant", status: "failed", error_code: "cancelled" });
  page.conversation.active_turn = null;
  await waitFor(() => expect(sourceReads).toBe(1), { timeout: 3500 });
  expect(send).not.toHaveBeenCalled();
  // No pending work remains; another poll period makes no source request.
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 2100)); });
  expect(sourceReads).toBe(1);
  expect(send).not.toHaveBeenCalled();
  f.unmount();
});

it.each([403, 404])("an off-selection %s fences cards queued during the same source read", async (status) => {
  const f = decisionNavigationFixture(true);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  vi.useFakeTimers();
  const history = vi.spyOn(nyxAgentTransport, "history");
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  const decideSource = f.result.current.decideAcknowledgement;
  await act(async () => {
    const decision = decideSource({ id: f.card.id, choice: "allow" });
    f.resolve(); await decision;
    await vi.advanceTimersByTimeAsync(0);
  });
  // The selected source uses only its existing two-second Query observer.
  history.mockClear();
  await act(() => vi.advanceTimersByTimeAsync(2000));
  expect(history).toHaveBeenCalledExactlyOnceWith(id, undefined, null);
  await act(async () => {
    f.rerender({ selected: f.second, enabled: true });
    await vi.advanceTimersByTimeAsync(0);
  });
  const secondCard = { ...f.card, id: "12345678-1234-4123-8123-123456789013", status: "allowed", decided_by: "user" };
  let refuse!: (response: Response) => void;
  const pendingRead = new Promise<Response>((resolve) => { refuse = resolve; });
  let sourceReads = 0;
  const mock = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (r) => {
    if (r.endpoint.endsWith(`/acknowledgements/${secondCard.id}`)) return json(secondCard);
    if (r.endpoint.includes(`/conversations/${id}`)) { sourceReads++; return pendingRead; }
    return mock(r);
  };
  await act(() => vi.advanceTimersByTimeAsync(2000));
  expect(sourceReads).toBe(1);
  await act(async () => {
    await decideSource({ id: secondCard.id, choice: "allow" });
    await vi.advanceTimersByTimeAsync(4000);
  });
  expect(sourceReads).toBe(1); // No overlapping source read after another card joins.
  await act(async () => {
    refuse(new Response(null, { status }));
    await vi.advanceTimersByTimeAsync(0);
  });
  page.conversation.active_turn = null;
  globalThis.__nyxidAssistantHttpMock = mock;
  await act(() => vi.advanceTimersByTimeAsync(4000));
  expect(sourceReads).toBe(1);
  expect(send).not.toHaveBeenCalled();
  f.unmount();
});

it.each([403, 404, 500])("source-owned history reads fence late %s across navigation and unmount", async (status) => {
  for (const entryPoint of ["refreshHistory", "loadOlder", "transport"] as const) {
    for (const leave of ["navigate", "unmount"] as const) {
      page.before_seq = 1;
      page.conversation.active_turn = { turn_id: "turn", started_at: "2026-09-17T00:00:00Z", activities: [], attachments: [] };
      const f = decisionNavigationFixture(true);
      await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
      const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
      await act(async () => {
        const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
        f.resolve(); await decision;
      });
      let release!: (response: Response) => void;
      const pending = new Promise<Response>((resolve) => { release = resolve; });
      const mock = globalThis.__nyxidAssistantHttpMock!;
      let reads = 0;
      globalThis.__nyxidAssistantHttpMock = (r) => {
        if (r.endpoint.includes(`/conversations/${id}`) && r.init.method === "GET") { reads++; return pending; }
        return mock(r);
      };
      let read!: Promise<unknown>;
      act(() => {
        read = (entryPoint === "transport" ? nyxAgentTransport.history(id) : f.result.current[entryPoint]()).catch(() => undefined);
      });
      await waitFor(() => expect(reads).toBe(1));
      if (leave === "unmount") f.unmount();
      else f.rerender({ selected: f.second, enabled: true });
      await act(async () => { release(new Response(null, { status })); await read; });
      globalThis.__nyxidAssistantHttpMock = mock;
      page.conversation.active_turn = null;
      await act(() => nyxAgentTransport.history(id));
      expect(send.mock.calls.length, `${entryPoint} ${leave}`).toBe(status === 500 ? 1 : 0);
      f.unmount();
      nyxAgentTransport.clear();
      send.mockRestore();
    }
  }
});

it.each(["actor", "authority"])("a late prior %s refusal cannot poison new queued work on the same source", async (transition) => {
  const f = decisionNavigationFixture(true);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  const complete = vi.spyOn(nyxAgentDecisions, "complete");
  let release!: (response: Response) => void;
  const pending = new Promise<Response>((resolve) => { release = resolve; });
  const mock = globalThis.__nyxidAssistantHttpMock!;
  let first = true;
  const secondCard: NyxAgentAcknowledgement = { ...f.card, id: "12345678-1234-4123-8123-123456789013", status: "allowed", decided_by: "user" };
  globalThis.__nyxidAssistantHttpMock = (r) => {
    if (r.endpoint.endsWith(`/acknowledgements/${secondCard.id}`)) return json(secondCard);
    if (r.endpoint.includes(`/conversations/${id}`) && r.init.method === "GET" && first) {
      first = false; return pending;
    }
    return mock(r);
  };
  let read!: Promise<unknown>;
  act(() => { read = f.result.current.refreshHistory(); });
  await waitFor(() => expect(first).toBe(false));
  if (transition === "actor") {
    const owner = useAuthStore.getState().user!;
    act(() => {
      useAuthStore.getState().setUser({ ...owner, id: "another-owner" });
      useAuthStore.getState().setUser(owner);
    });
  } else {
    f.rerender({ selected: id, enabled: false });
    f.rerender({ selected: id, enabled: true });
  }
  page.acknowledgements = [secondCard];
  await act(() => nyxAgentTransport.history(id));
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: secondCard.id, choice: "allow" }); });
  // The new decision is queued before its invalidation waits for the old read.
  await waitFor(() => expect(complete).toHaveBeenCalledWith(expect.anything(), "allow", secondCard));
  await act(async () => { release(new Response(null, { status: 403 })); await read; await decision; });
  expect(send).not.toHaveBeenCalled();
  page.conversation.active_turn = null;
  await act(() => nyxAgentTransport.history(id));
  expect(send).toHaveBeenCalledOnce();
  expect(send.mock.calls[0]?.[4]).toEqual(expect.objectContaining({ acknowledgementId: secondCard.id }));
  f.unmount();
});

it.each([false, true])("does not resume when Stop settles before an in-flight Allow returns (later message: %s)", async (laterMessage) => {
  const f = decisionNavigationFixture(true);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" }); });
  await waitFor(() => expect(requests.some((r) => r.endsWith(`/acknowledgements/${f.card.id}`))).toBe(true));
  f.rerender({ selected: f.second, enabled: true });
  page.messages.push({ ...page.messages[0]!, id: "stopped", seq: 2, role: "assistant", status: "failed", error_code: "cancelled" });
  if (laterMessage) page.messages.push({ ...page.messages[0]!, id: "unrelated", turn_id: "another-turn", seq: 3 });
  page.conversation.active_turn = null;
  await act(() => nyxAgentTransport.history(id));
  await act(async () => { f.resolve(); await decision; });
  expect(send).not.toHaveBeenCalled();
  f.unmount();
});

it("a queued turn's Stop suppresses only the cards bound to that turn", async () => {
  const f = decisionNavigationFixture(true);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  let now = Date.now();
  vi.spyOn(Date, "now").mockImplementation(() => now);
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  await act(async () => {
    const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
    f.resolve(); await decision;
  });
  page.conversation.active_turn = { ...page.conversation.active_turn!, turn_id: "next-turn" };
  page.messages.push({ ...page.messages[0]!, id: "completed", seq: 2, role: "assistant" });
  page.messages.push({ ...page.messages[0]!, id: "next-user", turn_id: "next-turn", seq: 3 });
  const secondCard = { ...f.card, id: "12345678-1234-4123-8123-123456789013" };
  page.acknowledgements = [{ ...f.card, status: "allowed", decided_by: "user" }, secondCard];
  await act(() => nyxAgentTransport.history(id));
  const mock = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (!request.endpoint.endsWith(`/acknowledgements/${secondCard.id}`)) return mock(request);
    const allowed = { ...secondCard, status: "allowed" as const, decided_by: "user" as const };
    page.acknowledgements = page.acknowledgements.map((row) => row.id === secondCard.id ? allowed : row);
    return json(allowed);
  };
  now += 750;
  await act(() => f.result.current.decideAcknowledgement({ id: secondCard.id, choice: "allow" }));
  expect(send).not.toHaveBeenCalled();
  page.conversation.active_turn = null;
  page.messages.push({ ...page.messages[0]!, id: "next-stopped", turn_id: "next-turn", seq: 4, role: "assistant", status: "failed", error_code: "cancelled" });
  await act(() => nyxAgentTransport.history(id));
  expect(send).toHaveBeenCalledOnce();
  expect(send.mock.calls[0]?.[1]).toContain(f.card.id);
  expect(send.mock.calls[0]?.[1]).not.toContain(secondCard.id);
  expect(send.mock.calls[0]?.[4]).toMatchObject({ acknowledgementId: f.card.id });
  f.unmount();
});

it("resumes a valid explicit Allow clicked after an earlier Stop has settled", async () => {
  page.messages.push({ ...page.messages[0]!, id: "earlier-stop", seq: 2, role: "assistant", status: "failed", error_code: "cancelled" });
  const f = decisionNavigationFixture(false);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  await act(async () => {
    const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
    f.resolve(); await decision;
  });
  expect(send).toHaveBeenCalledOnce();
  expect(send.mock.calls[0]?.[0]).toBe(id);
  f.unmount();
});

it.each([false, true])("an unrelated turn's cancellation cannot suppress Allow (queued: %s)", async (queued) => {
  const f = decisionNavigationFixture(queued);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" }); });
  await waitFor(() => expect(requests.some((r) => r.endsWith(`/acknowledgements/${f.card.id}`))).toBe(true));
  if (queued) {
    await act(async () => { f.resolve(); await decision; });
    expect(send).not.toHaveBeenCalled();
    f.rerender({ selected: f.second, enabled: true });
  }
  page.conversation.active_turn = null;
  page.messages.push({ ...page.messages[0]!, id: "other-stop", turn_id: "another-turn", seq: 2, role: "assistant", status: "failed", error_code: "cancelled" });
  if (!queued) {
    await act(() => nyxAgentTransport.history(id));
    await act(async () => { f.resolve(); await decision; });
  }
  await waitFor(() => expect(send).toHaveBeenCalledOnce(), { timeout: 3500 });
  expect(send.mock.calls[0]?.[0]).toBe(id);
  f.unmount();
});

it("binds the source before asynchronous mutation middleware", async () => {
  let release!: () => void;
  const middleware = new Promise<void>((resolve) => { release = resolve; });
  client.setDefaultOptions({ mutations: { onMutate: () => middleware } });
  const f = decisionNavigationFixture(false);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  let decision!: Promise<unknown>;
  act(() => { decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" }); });
  f.rerender({ selected: f.second, enabled: true });
  await act(async () => { release(); f.resolve(); await decision; });
  expect(requests.filter((r) => r.includes("/acknowledgements/"))).toEqual([
    `POST /assistant/nyxagent/conversations/${id}/acknowledgements/${f.card.id}`,
  ]);
  expect(send.mock.calls[0]?.[0]).toBe(id);
  f.unmount();
});

it.each([403, 404])("a selected thread's %s cannot erase another thread's valid continuation", async (status) => {
  const f = decisionNavigationFixture(true, status);
  await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
  await act(async () => {
    const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
    f.resolve(); await decision;
  });
  f.rerender({ selected: f.second, enabled: true });
  await waitFor(() => expect(f.result.current.historyDenied).toBe(true));
  expect(send).not.toHaveBeenCalled();
  page.conversation.active_turn = null;
  await waitFor(() => expect(send).toHaveBeenCalledOnce(), { timeout: 3500 });
  expect(send.mock.calls[0]?.[0]).toBe(id);
  f.unmount();
});

it.each(["actor", "actor round trip", "disabled", "source denied", "Stop"])(
  "discards a queued continuation after %s", async (transition) => {
    const f = decisionNavigationFixture(true);
    await waitFor(() => expect(f.result.current.acknowledgements).toHaveLength(1));
    const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue(undefined);
    await act(async () => {
      const decision = f.result.current.decideAcknowledgement({ id: f.card.id, choice: "allow" });
      f.resolve(); await decision;
    });
    expect(send).not.toHaveBeenCalled();
    if (transition.startsWith("actor")) {
      const owner = useAuthStore.getState().user!;
      act(() => {
        useAuthStore.getState().setUser({ ...owner, id: "another-owner" });
        if (transition === "actor round trip") useAuthStore.getState().setUser(owner);
      });
    } else if (transition === "disabled") {
      f.rerender({ selected: id, enabled: false });
    } else if (transition === "source denied") {
      const mock = globalThis.__nyxidAssistantHttpMock!;
      globalThis.__nyxidAssistantHttpMock = (r) =>
        r.endpoint.includes(`/conversations/${id}`) ? new Response(null, { status: 403 }) : mock(r);
      await act(() => f.result.current.refreshHistory());
      await waitFor(() => expect(f.result.current.historyDenied).toBe(true));
    } else {
      page.messages.push({ ...page.messages[0]!, id: "stopped", seq: 2, role: "assistant", status: "failed", error_code: "cancelled" });
    }
    page.conversation.active_turn = null;
    if (transition !== "source denied") await act(() => nyxAgentTransport.history(id));
    f.rerender({ selected: id, enabled: true });
    expect(send).not.toHaveBeenCalled();
    f.unmount();
  },
);
it("does not poll an idle thread while its agent works elsewhere, and re-reads it when the agent settles", async () => {
  const specialist = `nyxa-${"b".repeat(32)}`;
  page.conversation.active_turn = null;
  const specialistPage = {
    ...page,
    conversation: {
      ...page.conversation, id: specialist, role: "subagent" as const,
      agent: { id: "agent-researcher", kind: "specialist" as const, name: "researcher", destroyed: false },
    },
  };
  agents = [
    agentRow({}),
    agentRow({ id: "agent-researcher", kind: "specialist", name: "researcher", status: "running" }),
  ];
  const mock = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (request.endpoint.includes(`/conversations/${specialist}`)) {
      requests.push(`GET ${request.endpoint}`);
      return json(specialistPage);
    }
    return mock(request);
  };
  const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: specialist, onConversationAdopted: vi.fn(),
  }), { wrapper });
  await waitFor(() => expect(result.current.conversation?.agent?.name).toBe("researcher"));
  const reads = () => requests.filter((r) => r.includes(`/conversations/${specialist}?`)).length;
  // A Telegram chat or another thread keeps the agent busy; this thread has no turn.
  const first = reads();
  await new Promise((resolve) => setTimeout(resolve, 2500));
  expect(reads()).toBe(first);
  // The agent settles (the agents list polls while one runs): NyxID may have
  // written to this thread, so it is read once more.
  agents = [
    agentRow({}),
    agentRow({
      id: "agent-researcher", kind: "specialist", name: "researcher", status: "idle",
      last_reply: { seq: 7, status: "completed", text: "Done", created_at: "2026-09-17T00:01:00Z" },
    }),
  ];
  await waitFor(() => expect(reads()).toBe(first + 1), { timeout: 4500 });
  unmount();
}, 12000);

it("re-reads the open thread when a specialist settles, since NyxBot may have been woken", async () => {
  page.conversation.active_turn = null;
  agents = [
    agentRow({}),
    agentRow({ id: "agent-researcher", kind: "specialist", name: "researcher", status: "running" }),
  ];
  const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: id, onConversationAdopted: vi.fn(),
  }), { wrapper });
  await waitFor(() => expect(result.current.session.title).toBe("Question"));
  const reads = () => requests.filter((r) => r.includes(`/conversations/${id}?`)).length;
  const before = reads();
  // The next agents poll finds the researcher done with a new reply.
  agents = [
    agentRow({}),
    agentRow({
      id: "agent-researcher", kind: "specialist", name: "researcher", status: "idle",
      last_reply: { seq: 5, status: "completed", text: "Done", created_at: "2026-09-17T00:00:05Z" },
    }),
  ];
  await waitFor(() => expect(reads()).toBeGreaterThan(before), { timeout: 5000 });
  unmount();
}, 10_000);

it("names the agent only when a send starts a new thread", async () => {
  page.conversation.active_turn = null;
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue();
  const researcher = { id: "agent-researcher", kind: "specialist" as const, name: "researcher", destroyed: false };
  const draft = renderHook(() => useNyxAgentAssistantChat({
    onConversationAdopted: vi.fn(), draftAgent: researcher,
  }), { wrapper });
  await act(() => draft.result.current.send("Hello researcher"));
  expect(send).toHaveBeenLastCalledWith(
    undefined, "Hello researcher", expect.any(Function), expect.any(Function), { agent: researcher },
  );
  draft.unmount();
  const existing = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: id, onConversationAdopted: vi.fn(), draftAgent: researcher,
  }), { wrapper });
  await act(() => existing.result.current.send("Continue"));
  expect(send.mock.lastCall).toHaveLength(4);
  expect(send.mock.lastCall?.[0]).toBe(id);
  existing.unmount();
});

it("passes an upload's attachment ids and metadata to the transport", async () => {
  page.conversation.active_turn = null;
  const send = vi.spyOn(nyxAgentTransport, "send").mockResolvedValue();
  const photo = { id: "img-1", content_type: "image/png" as const, size: 4, label: "photo.png", origin: "user_upload" as const };
  const draftId = `nyxa-${"d".repeat(32)}`;
  // A new thread: the composer created an upload draft, which the turn adopts.
  const fresh = renderHook(() => useNyxAgentAssistantChat({
    onConversationAdopted: vi.fn(), draftAgent: nyxbotRef,
  }), { wrapper });
  await act(() => fresh.result.current.send("Look", {
    attachmentIds: [photo.id], attachments: [photo], conversationId: draftId,
  }));
  expect(send).toHaveBeenLastCalledWith(draftId, "Look", expect.any(Function), expect.any(Function), {
    attachmentIds: [photo.id], attachments: [photo], adoptExisting: true,
  });
  fresh.unmount();
  const existing = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: id, onConversationAdopted: vi.fn(), draftAgent: nyxbotRef,
  }), { wrapper });
  await act(() => existing.result.current.send("", {
    attachmentIds: [photo.id], attachments: [photo], conversationId: id,
  }));
  expect(send).toHaveBeenLastCalledWith(id, "", expect.any(Function), expect.any(Function), {
    attachmentIds: [photo.id], attachments: [photo], adoptExisting: false,
  });
  existing.unmount();
});

it("shows a new thread's sent image at once, before the draft is adopted", async () => {
  const photo = { id: "img-1", content_type: "image/png" as const, size: 4, label: "photo.png", origin: "user_upload" as const };
  const draftId = `nyxa-${"d".repeat(32)}`;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  const draftPage: NyxAgentHistory = {
    ...page,
    conversation: { ...page.conversation, id: draftId, active_turn: null },
    messages: [{ ...page.messages[0]!, text: "Look", attachments: [photo] }],
  };
  const defaultMock = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = async (request) => {
    if (request.endpoint.endsWith("/turns")) {
      await gate;
      const events = [
        { event: "turn.status", conversation_id: draftId, turn_id: "turn", status: "running" },
        { event: "turn.completed", turn_id: "turn", status: "completed", error: null },
      ];
      return new Response(events.map((event, i) => `data: ${JSON.stringify({ ...event, cursor: i + 1 })}\n\n`).join(""));
    }
    if (request.endpoint.includes(`/conversations/${draftId}`)) return json(draftPage);
    return defaultMock(request);
  };
  const adopted = vi.fn();
  const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
    onConversationAdopted: adopted, draftAgent: nyxbotRef,
  }), { wrapper });
  let sending!: Promise<void>;
  act(() => {
    sending = result.current.send("Look", {
      attachmentIds: [photo.id], attachments: [photo], conversationId: draftId,
    });
  });
  await waitFor(() => expect(result.current.session.messages[0]?.images).toEqual([{
    id: "img-1",
    endpoint: `/assistant/nyxagent/conversations/${draftId}/attachments/img-1`,
    contentType: "image/png",
    label: "photo.png",
    origin: "user_upload",
  }]));
  expect(result.current.session.messages[0]).toMatchObject({ role: "user", content: "Look" });
  // Still a new thread to the page: no conversation id until adoption.
  expect(result.current.session.conversationId).toBeUndefined();
  expect(result.current.isStreaming).toBe(true);
  expect(adopted).not.toHaveBeenCalled();
  release();
  await act(() => sending);
  expect(adopted).toHaveBeenCalledExactlyOnceWith(draftId);
  // Unadopted here (the page would navigate): the draft view ends with its send.
  expect(result.current.isStreaming).toBe(false);
  expect(result.current.session.messages).toEqual([]);
  unmount();
});

it("phrases continuation turns per acknowledgement kind", () => {
  const base = {
    id: "12345678-1234-4123-8123-123456789012",
    status: "allowed" as const,
    summary: "Delete agent key ci-bot",
    decider: "user" as const,
    decided_by: "user" as const,
    reason: null,
    service_slug: null,
    service_name: null,
    tool_name: null,
    created_at: "2026-09-17T00:00:00Z",
    decided_at: "2026-09-17T00:00:01Z",
    expires_at: "2099-09-17T00:15:00Z",
  };
  expect(
    continuationText({ ...base, kind: "service", service_slug: "github", service_name: "GitHub" }),
  ).toBe("Approved: this chat may use GitHub. Continue. Then continue the original task and tell the user when it is finished or when you need their input.");
  expect(continuationText({ ...base, kind: "service", service_slug: "github" })).toBe(
    "Approved: this chat may use github. Continue. Then continue the original task and tell the user when it is finished or when you need their input.",
  );
  expect(continuationText({ ...base, kind: "account" })).toBe(
    "Approved: account management for this chat. Continue. Then continue the original task and tell the user when it is finished or when you need their input.",
  );
  expect(continuationForAll([{ ...base, kind: "account" }])).toBe(
    "Approved: account management for this chat. Continue. Then continue the original task and tell the user when it is finished or when you need their input.",
  );
  expect(continuationText({ ...base, kind: "action", tool_name: "delete_agent_key" })).toBe(
    `Confirmed: Delete agent key ci-bot (acknowledgement_id ${base.id}). Retry it now. Then continue the original task and tell the user when it is finished or when you need their input.`,
  );
  for (const kind of ["machine", "saved_login"] as const) {
    expect(continuationText({ ...base, kind })).toBe(
      "Approved: Delete agent key ci-bot Continue within the updated machine and saved-login grants. Then continue the original task and tell the user when it is finished or when you need their input.",
    );
  }
  // Only a routed skills card (NyxBot decides) wakes NyxBot after an allow.
  expect(continuationText({ ...base, kind: "skills", decider: "orchestrator" })).toBe(
    "Skill proposal reviewed: Delete agent key ci-bot NyxBot will resolve the exact version and apply the governing mode to attach it (automatic in Full access, otherwise an owner card). Do not request the skill again. Then continue the original task and tell the user when it is finished or when you need their input.",
  );
  const unrouted = continuationText({ ...base, kind: "skills" });
  expect(unrouted).toBe(
    "Skill proposal reviewed: Delete agent key ci-bot Nothing was attached and nobody else was asked; the user adds the skill from this agent's Skills editor. Do not request the skill again. Then continue the original task and tell the user when it is finished or when you need their input.",
  );
  expect(unrouted).not.toMatch(/NyxBot/);
});

function failedReply(turnId: string, code: string, seq = 2) {
  return {
    id: `assistant-${turnId}`,
    turn_id: turnId,
    seq,
    role: "assistant" as const,
    text: "",
    status: "failed" as const,
    error_code: code,
    created_at: "2026-09-17T00:00:01Z",
    activities: [],
    attachments: [],
  };
}

function sse(events: readonly Record<string, unknown>[]) {
  return new Response(
    events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""),
    { headers: { "Content-Type": "text/event-stream" } },
  );
}

it("opens the credits dialog once when a live turn fails, across SSE and the poll", async () => {
  useCreditsDenialStore.getState().reset();
  page.conversation.active_turn = null;
  const mock = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (request.endpoint === "/assistant/nyxagent/turns") {
      page.messages.push(failedReply("live", "insufficient_credits"));
      return sse([
        {
          cursor: 1,
          event: "turn.status",
          conversation_id: id,
          turn_id: "live",
          status: "running",
        },
        {
          cursor: 2,
          event: "turn.completed",
          turn_id: "live",
          status: "failed",
          error: { code: "insufficient_credits", message: "No credits" },
        },
      ]);
    }
    return mock(request);
  };
  const { result, unmount } = renderHook(
    () =>
      useNyxAgentAssistantChat({
        selectedConversationId: id,
        onConversationAdopted: vi.fn(),
      }),
    { wrapper },
  );
  await waitFor(() => expect(result.current.isLoading).toBe(false));
  await act(() => result.current.send("Question"));
  expect(useCreditsDenialStore.getState().current).toMatchObject({
    key: `assistant:nyxagent:${id}:live`,
    payer: "self",
  });
  useCreditsDenialStore.getState().dismiss();
  // The refreshed history now also shows the failure; it must not reopen.
  await act(() => client.invalidateQueries());
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(useCreditsDenialStore.getState().current).toBeNull();
  unmount();
});

it("opens for a turn seen running that the history poll reports failed", async () => {
  useCreditsDenialStore.getState().reset();
  const { result, unmount } = renderHook(
    () =>
      useNyxAgentAssistantChat({
        selectedConversationId: id,
        onConversationAdopted: vi.fn(),
      }),
    { wrapper },
  );
  await waitFor(() => expect(result.current.isStreaming).toBe(true));
  page.conversation.active_turn = null;
  page.messages.push(failedReply("turn", "insufficient_credits"));
  await waitFor(
    () =>
      expect(useCreditsDenialStore.getState().current?.key).toBe(
        `assistant:nyxagent:${id}:turn`,
      ),
    { timeout: 4000 },
  );
  unmount();
}, 8000);

it("never opens for failures already in the transcript or in older pages", async () => {
  useCreditsDenialStore.getState().reset();
  page.conversation.active_turn = null;
  page.messages.push(failedReply("old", "insufficient_credits"));
  page.before_seq = 1;
  const { result, unmount } = renderHook(
    () =>
      useNyxAgentAssistantChat({
        selectedConversationId: id,
        onConversationAdopted: vi.fn(),
      }),
    { wrapper },
  );
  await waitFor(() =>
    expect(result.current.session.messages.at(-1)?.status).toBe("error"),
  );
  expect(result.current.session.messages.at(-1)?.error).toBe(
    "There aren't enough credits to run this turn. Add credits in Billing & Usage, then try again.",
  );
  page.messages = [failedReply("older", "insufficient_credits", 1)];
  page.before_seq = null;
  await act(() => result.current.loadOlder());
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(useCreditsDenialStore.getState().current).toBeNull();
  unmount();
});

it("ignores a stale index that still shows an already-failed turn as running", async () => {
  useCreditsDenialStore.getState().reset();
  page.conversation.active_turn = null;
  page.messages.push(failedReply("turn", "insufficient_credits"));
  const stale = {
    ...page.conversation,
    active_turn: { turn_id: "turn", started_at: "2026-09-17T00:00:00Z", activities: [], attachments: [] },
  };
  let releaseIndex!: () => void;
  const indexGate = new Promise<void>((resolve) => (releaseIndex = resolve));
  let indexServed = false;
  const mock = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = async (request) => {
    if (request.endpoint.includes("/conversations?limit=")) {
      await indexGate;
      indexServed = true;
      return json({ conversations: [stale], next_cursor: null });
    }
    return mock(request);
  };
  const { result, unmount } = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: id, onConversationAdopted: vi.fn(), threadsAgentId: NYXBOT,
  }), { wrapper });
  await waitFor(() => expect(result.current.session.messages.at(-1)?.status).toBe("error"));
  // The slower index response lands after the idle history.
  releaseIndex();
  await waitFor(() => expect(indexServed).toBe(true));
  await waitFor(() => expect(nyxAgentTransport.getConversations()[0]?.active_turn?.turn_id).toBe("turn"));
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(useCreditsDenialStore.getState().current).toBeNull();
  unmount();
});

it("switching to another idle thread re-reads neither the thread list nor the agents", async () => {
  page.conversation.active_turn = null;
  const second = `nyxa-${"c".repeat(32)}`;
  const mock = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = (request) => {
    if (request.endpoint.includes(`/conversations/${second}`)) {
      requests.push(`GET ${request.endpoint}`);
      return json({ ...page, conversation: { ...page.conversation, id: second } });
    }
    return mock(request);
  };
  const { result, rerender, unmount } = renderHook(
    ({ selected }: { selected: string }) => useNyxAgentAssistantChat({
      selectedConversationId: selected, threadsAgentId: NYXBOT, onConversationAdopted: vi.fn(),
    }),
    { wrapper, initialProps: { selected: id } },
  );
  await waitFor(() => expect(result.current.conversation?.id).toBe(id));
  await new Promise((resolve) => setTimeout(resolve, 300));
  const lists = () => requests.filter((r) => r.startsWith("GET /assistant/nyxagent/conversations?")).length;
  const agentReads = () => requests.filter((r) => r.startsWith("GET /assistant/nyxagent/agents")).length;
  const before = { lists: lists(), agents: agentReads() };
  rerender({ selected: second });
  await waitFor(() => expect(result.current.conversation?.id).toBe(second));
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect({ lists: lists(), agents: agentReads() }).toEqual(before);
  unmount();
});

it("the page and the sidebar refresh the thread list once per agents change", async () => {
  page.conversation.active_turn = null;
  const { result, unmount } = renderHook(() => ({
    page: useNyxAgentAssistantChat({
      selectedConversationId: id, threadsAgentId: NYXBOT, onConversationAdopted: vi.fn(),
    }),
    sidebar: useNyxAgentThreads({ agentId: NYXBOT }),
  }), { wrapper });
  await waitFor(() => expect(result.current.sidebar.threadsLoaded).toBe(true));
  await new Promise((resolve) => setTimeout(resolve, 300));
  const lists = () => requests.filter((r) => r.startsWith("GET /assistant/nyxagent/conversations?")).length;
  const before = lists();
  agents = [agentRow({ last_reply: { seq: 3, status: "completed", text: "Done", created_at: "2026-09-17T00:01:00Z" } })];
  await act(() => client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents("owner") }));
  await waitFor(() => expect(lists()).toBe(before + 1));
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect(lists()).toBe(before + 1);
  unmount();
});

it("2345 malformed history is friendly and refresh clears the query error", async () => {
  const goodMessages = page.messages;
  page.messages = [{ text: "private" }] as unknown as NyxAgentHistory["messages"];
  const { result } = renderHook(() => useNyxAgentAssistantChat({
    selectedConversationId: id, onConversationAdopted: vi.fn(), threadsAgentId: NYXBOT,
  }), { wrapper });
  await waitFor(() => expect(result.current.error).toBe("Conversation history could not be loaded. Please try again."));
  page.messages = goodMessages;
  await act(() => result.current.refreshHistory());
  await waitFor(() => expect(result.current.error).toBeUndefined());
  expect(result.current.conversation?.id).toBe(id);
});
