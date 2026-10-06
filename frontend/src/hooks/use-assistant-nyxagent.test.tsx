import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { continuationForAll, continuationText, useNyxAgentAssistantChat } from "./use-assistant-nyxagent";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { useAuthStore } from "@/stores/auth-store";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";
import type { NyxAgentHistory } from "@/schemas/assistant-nyxagent";

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
      decider: "user" as const, decided_by: null, reason: null,
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

it.each(["specialist", "automation", "voice"])(
  "leaves %s confirmation continuation to the server",
  async (kind) => {
    page.conversation.active_turn = null;
    page.conversation.pending_acknowledgements = 1;
    page.acknowledgements = [
      {
        id: "12345678-1234-4123-8123-123456789012",
        kind: "service",
        status: "pending",
        summary: "Use GitHub",
        decider: kind === "specialist" ? "orchestrator" : "user",
        trigger_run_id: kind === "automation" ? "run-id" : null,
        continuation_owner: kind === "voice" ? "server" : null,
        continuation_receipt_id: kind === "voice" ? "12345678-1234-4123-8123-123456789013" : null,
        decided_by: null,
        reason: null,
        service_slug: "github",
        service_name: "GitHub",
        tool_name: null,
        created_at: "2026-09-17T00:00:00Z",
        decided_at: null,
        expires_at: "2026-09-17T00:15:00Z",
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
it("polls a thread whose agent NyxID set to work, without a turn of its own", async () => {
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
  const first = reads();
  await waitFor(() => expect(reads()).toBeGreaterThan(first), { timeout: 3500 });
  unmount();
}, 8000);

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
    "There aren't enough credits to run this turn.",
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
