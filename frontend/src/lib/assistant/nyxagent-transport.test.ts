import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NyxAgentTransport, CONTEXT_RESET_NOTICE } from "./nyxagent-transport";
import { transitionAssistantIdentity } from "./identity";
import { assistantChatSurface } from "./conversation-ids";
import type { NyxAgentConversation, NyxAgentHistory } from "@/schemas/assistant-nyxagent";

const id = `nyxa-${"1".repeat(32)}`;
const other = `nyxa-${"2".repeat(32)}`;
const start = "2026-09-17T00:00:00Z";
const reset = "2026-09-17T00:00:01Z";
const afterReset = "2026-09-17T00:00:02Z";

function conversation(key = id): NyxAgentConversation {
  return {
    id: key,
    title: "Question",
    model: "nyxagent/chat",
    access_mode: "ask",
    created_at: start,
    last_message_at: start,
    message_count: 2,
    pending_acknowledgements: 0,
    active_turn: null,
    context_reset_at: null,
  };
}

function history(): NyxAgentHistory {
  return {
    conversation: conversation(),
    messages: [
      {
        id: "u",
        seq: 1,
        turn_id: "turn",
        role: "user",
        text: "Question",
        status: "completed",
        error_code: null,
        created_at: start,
        activities: [],
      },
      {
        id: "a",
        seq: 2,
        turn_id: "turn",
        role: "assistant",
        text: "Answer 界",
        status: "completed",
        error_code: null,
        created_at: afterReset,
        activities: [],
      },
    ],
    before_seq: null,
    acknowledgements: [],
    approvals: [],
  };
}

const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });

function stream(): Response {
  const events = [
    { event: "turn.status", conversation_id: id, turn_id: "turn", status: "running" },
    { event: "message.started", message_id: "a", role: "assistant" },
    {
      event: "block.started",
      block_id: "b",
      message_id: "a",
      index: 0,
      block: { type: "text", block_id: "b", text: "" },
    },
    { event: "turn.notice", code: "context_reset", message: "ignore upstream text" },
    { event: "block.delta", block_id: "b", text: "Answer 界" },
    {
      event: "block.completed",
      block_id: "b",
      block: { type: "text", block_id: "b", text: "Answer 界" },
    },
    { event: "message.completed", message_id: "a" },
    { event: "turn.completed", turn_id: "turn", status: "completed", error: null },
  ];
  const bytes = new TextEncoder().encode(
    events
      .map((event, i) => `data: ${JSON.stringify({ ...event, cursor: i + 1 })}\r\n\r\n`)
      .join(""),
  );
  let cursor = 0;
  return new Response(
    new ReadableStream({
      pull(controller) {
        if (cursor === bytes.length) {
          controller.close();
          return;
        }
        controller.enqueue(bytes.slice(cursor, ++cursor));
      },
    }),
    { headers: { "content-type": "text/event-stream" } },
  );
}

beforeEach(() => {
  transitionAssistantIdentity(null);
  transitionAssistantIdentity("owner");
});

afterEach(() => {
  globalThis.__nyxidAssistantHttpMock = undefined;
  transitionAssistantIdentity(null);
  vi.restoreAllMocks();
});

describe("NyxAgent server-backed transport", () => {
  it("adopts server identity, decodes fragmented SSE and a live inline notice, then reads history", async () => {
    const requests: { endpoint: string; body?: BodyInit | null }[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      requests.push({ endpoint, body: init.body });
      if (endpoint.endsWith("/turns")) return stream();
      if (endpoint.includes(`/conversations/${id}`)) {
        const page = history();
        page.conversation.context_reset_at = reset;
        return json(page);
      }
      return json({ conversations: [conversation()], next_cursor: null });
    };
    const transport = new NyxAgentTransport();
    const adopted = vi.fn();
    let sawLiveNotice = false;
    transport.subscribe(() => {
      if (transport.isRunning(id)) {
        sawLiveNotice ||= transport.session(id).messages.some((m) => m.role === "system");
      }
    });
    transport.setModel("nyxagent/research");
    await transport.send(undefined, "Question", adopted);
    expect(adopted).toHaveBeenCalledExactlyOnceWith(id);
    expect(sawLiveNotice).toBe(true);
    const messages = transport.session(id).messages;
    expect(messages.map((m) => m.role)).toEqual(["user", "system", "assistant"]);
    expect(messages[1]?.content).toBe(CONTEXT_RESET_NOTICE);
    expect(messages.at(-1)?.content).toBe("Answer 界");
    expect(transport.isRunning(id)).toBe(false);
    expect(JSON.parse(String(requests[0]?.body))).toEqual({
      text: "Question",
      model: "nyxagent/research",
      access_mode: "ask",
    });
    expect(requests.filter((r) => r.endpoint.endsWith("/turns"))).toHaveLength(1);
  });

  it.each([false, true])(
    "places one reset note after the failed reply (next turn present: %s)",
    async (nextTurn) => {
      const page = history();
      page.conversation.context_reset_at = reset;
      page.messages[1] = {
        ...page.messages[1]!,
        created_at: reset,
        status: "failed",
        error_code: "cancelled",
      };
      if (nextTurn) {
        page.messages.push({
          ...page.messages[0]!,
          id: "next-user",
          seq: 3,
          text: "Next",
          created_at: afterReset,
        });
      }
      globalThis.__nyxidAssistantHttpMock = () => json(page);
      const transport = new NyxAgentTransport();
      await transport.history(id);
      await transport.history(id);
      const messages = transport.session(id).messages;
      expect(messages.filter((m) => m.role === "system")).toHaveLength(1);
      expect(messages[2]?.content).toBe(CONTEXT_RESET_NOTICE);
      expect(messages[1]?.id).toBe("a");
      if (nextTurn) expect(messages[3]?.id).toBe("next-user");
      else expect(messages.at(-1)?.role).toBe("system");
    },
  );

  it("moves the single note to the latest reset before a rebound reply", async () => {
    const page = history();
    page.conversation.context_reset_at = start;
    globalThis.__nyxidAssistantHttpMock = () => json(page);
    const transport = new NyxAgentTransport();
    await transport.history(id);
    page.conversation.context_reset_at = reset;
    await transport.history(id);
    expect(transport.session(id).messages.map((m) => m.role)).toEqual([
      "user",
      "system",
      "assistant",
    ]);
  });

  it("drains index pages and merges transcript pages without duplicating a poll", async () => {
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint.includes(`/conversations/${id}`)) {
        const page = history();
        if (endpoint.includes("before_seq")) {
          page.messages = [page.messages[0]!];
        } else {
          page.messages = [page.messages[1]!];
          page.before_seq = 2;
        }
        return json(page);
      }
      return endpoint.includes("cursor=")
        ? json({ conversations: [conversation(other)], next_cursor: null })
        : json({ conversations: [conversation()], next_cursor: "next page" });
    };
    const transport = new NyxAgentTransport();
    expect(await transport.list()).toHaveLength(2);
    await transport.history(id);
    await transport.history(id, 2);
    await transport.history(id);
    expect(transport.getHistory(id)?.messages.map((m) => m.seq)).toEqual([1, 2]);
    expect(transport.getHistory(id)?.before_seq).toBeNull();
  });

  it("surfaces stable HTTP errors without treating them as an expired login", async () => {
    globalThis.__nyxidAssistantHttpMock = () =>
      json(
        {
          error: "turn_active",
          error_code: 12100,
          message: "A turn is already active",
        },
        409,
      );
    const transport = new NyxAgentTransport();
    await expect(transport.send(id, "Question", vi.fn())).rejects.toMatchObject({
      status: 409,
      message: "A turn is already active",
    });
    expect(transport.isRunning(id)).toBe(false);
  });

  it("reloads an active turn without resending and uses the owner-only stop endpoint", async () => {
    const requests: string[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      requests.push(`${init.method} ${endpoint}`);
      if (endpoint.endsWith("/stop")) return new Response(null, { status: 204 });
      const page = history();
      page.messages.pop();
      page.conversation.active_turn = {
        turn_id: "running",
        started_at: start,
        activities: [
          { id: "t1", label: "nyx__search_tools", status: "completed", started_at: start, ended_at: reset },
          { id: "t2", label: "github__list_issues", status: "running", started_at: reset, ended_at: null },
        ],
      };
      return json(page);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    expect(transport.isRunning(id)).toBe(true);
    const session = transport.session(id);
    expect(session.status).toBe("streaming");
    // Tool activity recorded on the live turn is shown on the streaming placeholder.
    expect(session.messages.at(-1)?.toolCalls).toEqual([
      { id: "t1", name: "nyx__search_tools", status: "done", startedAt: Date.parse(start), finishedAt: Date.parse(reset) },
      { id: "t2", name: "github__list_issues", status: "running", startedAt: Date.parse(reset), finishedAt: undefined },
    ]);
    await transport.stop(id);
    expect(requests).toHaveLength(2);
    expect(requests[1]).toBe(`POST /assistant/nyxagent/conversations/${id}/stop`);
  });

  it("clears identity caches and rejects late results for the previous owner", async () => {
    let resolve!: (value: Response) => void;
    globalThis.__nyxidAssistantHttpMock = () =>
      new Promise<Response>((done) => {
        resolve = done;
      });
    const transport = new NyxAgentTransport();
    const pending = transport.list();
    transitionAssistantIdentity("other-owner");
    resolve(json({ conversations: [conversation()], next_cursor: null }));
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
    expect(transport.getConversations()).toEqual([]);
    expect(transport.getModel()).toBe("nyxagent/chat");
  });

  it("renames and deletes through HTTP and evicts deleted transcript state", async () => {
    globalThis.__nyxidAssistantHttpMock = ({ init }) => {
      if (init.method === "DELETE") return new Response(null, { status: 204 });
      if (init.method === "PATCH") return json({ ...conversation(), title: "Renamed" });
      return json(history());
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    await transport.rename(id, "Renamed");
    expect(transport.session(id).title).toBe("Renamed");
    await transport.delete(id);
    expect(transport.getHistory(id)).toBeUndefined();
    expect(transport.getConversations()).toEqual([]);
  });

  it("rejects malformed SSE and never automatically retries a submitted message", async () => {
    let turns = 0;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint.endsWith("/turns")) {
        turns++;
        return new Response("data: broken\n\n");
      }
      return json({ conversations: [], next_cursor: null });
    };
    await expect(new NyxAgentTransport().send(undefined, "Question", vi.fn())).rejects.toThrow();
    expect(turns).toBe(1);
  });

  it("a stalled subscription refreshes history without sending Stop or replaying the turn", async () => {
    vi.useFakeTimers();
    const requests: string[] = [];
    const cancelled = vi.fn();
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      requests.push(endpoint);
      if (endpoint.endsWith("/turns")) {
        return new Response(
          new ReadableStream({
            start(controller) {
              const event = {
                event: "turn.status",
                cursor: 1,
                conversation_id: id,
                turn_id: "turn",
                status: "running",
              };
              controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`));
            },
            cancel: cancelled,
          }),
        );
      }
      const page = history();
      page.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [] };
      return endpoint.includes(`/conversations/${id}`)
        ? json(page)
        : json({ conversations: [page.conversation], next_cursor: null });
    };
    try {
      const transport = new NyxAgentTransport();
      const pending = transport.send(id, "Question", vi.fn());
      const rejected = expect(pending).rejects.toThrow("connection was interrupted");
      await vi.advanceTimersByTimeAsync(135_001);
      await rejected;
      expect(cancelled).toHaveBeenCalledOnce();
      expect(transport.isRunning(id)).toBe(true);
      expect(requests.filter((path) => path.endsWith("/turns"))).toHaveLength(1);
      expect(requests.some((path) => path.endsWith("/stop"))).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("late index and history reads cannot resurrect a deleted row", async () => {
    const transport = new NyxAgentTransport();
    globalThis.__nyxidAssistantHttpMock = () => json(history());
    await transport.history(id);
    let resolveIndex!: (value: Response) => void;
    let resolveHistory!: (value: Response) => void;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (init.method === "DELETE") return new Response(null, { status: 204 });
      return new Promise<Response>((done) => {
        if (endpoint.includes(`/conversations/${id}`)) resolveHistory = done;
        else resolveIndex = done;
      });
    };
    const indexRead = transport.list();
    const historyRead = transport.history(id);
    await transport.delete(id);
    resolveIndex(json({ conversations: [conversation()], next_cursor: null }));
    resolveHistory(json(history()));
    await indexRead;
    await expect(historyRead).rejects.toMatchObject({ name: "AbortError" });
    expect(transport.getConversations()).toEqual([]);
    expect(transport.getHistory(id)).toBeUndefined();
  });

  it("retains the uncertain-outcome warning when restoring a failed transcript", async () => {
    const page = history();
    page.messages[1] = { ...page.messages[1]!, status: "failed", error_code: "outcome_unknown" };
    globalThis.__nyxidAssistantHttpMock = () => json(page);
    const transport = new NyxAgentTransport();
    await transport.history(id);
    expect(transport.session(id).messages.at(-1)?.error).toContain("may have taken effect");
  });

  it("detects repeated index cursors", async () => {
    globalThis.__nyxidAssistantHttpMock = () => json({ conversations: [], next_cursor: "same" });
    await expect(new NyxAgentTransport().list()).rejects.toThrow("Invalid assistant pagination");
  });
});

describe("engine routing", () => {
  it("prefers NyxAgent, then Direct, then actor and preserves existing engines", () => {
    expect(
      assistantChatSurface({
        nyxagentEnabled: true,
        directEnabled: true,
        drafting: true,
      }),
    ).toBe("nyxagent");
    expect(
      assistantChatSurface({
        nyxagentEnabled: false,
        directEnabled: true,
        drafting: true,
      }),
    ).toBe("direct");
    expect(
      assistantChatSurface({
        nyxagentEnabled: false,
        directEnabled: false,
        drafting: true,
      }),
    ).toBe("actor");
    for (const selectedConversationId of ["nyxid-chat-old", "chatc-old"]) {
      expect(
        assistantChatSurface({
          nyxagentEnabled: true,
          directEnabled: true,
          drafting: false,
          selectedConversationId,
        }),
      ).toBe("actor");
    }
    expect(
      assistantChatSurface({
        nyxagentEnabled: true,
        directEnabled: true,
        drafting: false,
        selectedConversationId: id,
      }),
    ).toBe("nyxagent");
    expect(
      assistantChatSurface({
        nyxagentEnabled: false,
        directEnabled: true,
        drafting: false,
        selectedConversationId: id,
      }),
    ).toBe("nyxagent");
  });
});

it("keeps pending cards at the tail and decided cards beside the turn that requested them", async () => {
  const transport = new NyxAgentTransport();
  const page = history();
  const ack = {
    id: "12345678-1234-4123-8123-123456789012",
    kind: "service" as const,
    status: "pending" as const,
    summary: "Use GitHub",
    service_slug: "github",
    service_name: "GitHub",
    tool_name: null,
    created_at: reset,
    decided_at: null,
    expires_at: afterReset,
  };
  page.acknowledgements = [ack];
  page.conversation.pending_acknowledgements = 1;
  const writes: unknown[] = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (endpoint.includes("/acknowledgements/")) {
      writes.push(JSON.parse(String(init.body)) as unknown);
      return json({ ...ack, status: "allowed", decided_at: afterReset });
    }
    return json(page);
  };
  await transport.history(id);
  expect(transport.session(id).messages.map((m) => m.id)).toEqual([
    "u",
    "a",
    `nyxagent-acknowledgement:${ack.id}`,
  ]);
  await transport.decide(id, ack.id, "allow");
  expect(writes).toEqual([{ decision: "allow" }]);
  expect(transport.session(id).messages.map((m) => m.id)).toEqual([
    "u",
    `nyxagent-acknowledgement:${ack.id}`,
    "a",
  ]);
  expect(transport.getHistory(id)?.conversation.pending_acknowledgements).toBe(0);
  expect(transport.session(id).messages.filter((m) => m.role === "user")).toHaveLength(1);
});

it("does not adopt another identity's decision response", async () => {
  const transport = new NyxAgentTransport();
  globalThis.__nyxidAssistantHttpMock = () => {
    transitionAssistantIdentity("someone-else");
    return json({});
  };
  await expect(
    transport.decide(id, "12345678-1234-4123-8123-123456789012", "deny"),
  ).rejects.toThrow();
  expect(transport.getHistory(id)).toBeUndefined();
});

it("remembers the draft mode, sends it only at creation, and requires PATCH for existing chats", async () => {
  const transport = new NyxAgentTransport();
  const requests: { method: string; body: unknown }[] = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const body = init.body ? (JSON.parse(String(init.body)) as unknown) : null;
    requests.push({ method: init.method ?? "GET", body });
    if (endpoint.endsWith("/access-mode")) return json({ ...conversation(), access_mode: "full" });
    if (endpoint.endsWith("/turns")) return stream();
    if (endpoint.includes("/conversations?")) {
      return json({ conversations: [conversation()], next_cursor: null });
    }
    return json(history());
  };
  await transport.setAccessMode(undefined, "full");
  expect(requests).toHaveLength(0);
  expect(transport.getAccessMode()).toBe("full");
  await transport.history(id);
  expect(transport.getAccessMode(id)).toBe("ask");
  await transport.send(undefined, "Full draft", vi.fn());
  expect(requests.find((r) => r.method === "POST")?.body).toEqual({
    model: "nyxagent/chat",
    access_mode: "full",
    text: "Full draft",
  });
  await transport.setAccessMode(id, "full");
  expect(requests.find((r) => r.method === "PATCH")?.body).toEqual({ access_mode: "full" });
  expect(transport.getAccessMode(id)).toBe("full");
  requests.length = 0;
  await transport.send(id, "Continue", vi.fn());
  expect(requests.find((r) => r.method === "POST")?.body).toEqual({
    conversation_id: id,
    text: "Continue",
  });
});

it("retains a settled reply's tool activity as done tool calls", async () => {
  globalThis.__nyxidAssistantHttpMock = () => {
    const page = history();
    page.messages[1] = {
      ...page.messages[1]!,
      activities: [
        { id: "t1", label: "nyxid__list_agent_keys", status: "error", started_at: start, ended_at: reset },
      ],
    };
    return json(page);
  };
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const session = transport.session(id);
  expect(session.messages[0]?.toolCalls).toBeUndefined();
  expect(session.messages[1]?.toolCalls).toEqual([
    { id: "t1", name: "nyxid__list_agent_keys", status: "error", startedAt: Date.parse(start), finishedAt: Date.parse(reset) },
  ]);
});

it("shows pending proxy approvals raised by the chat key as cards at the tail", async () => {
  globalThis.__nyxidAssistantHttpMock = () => {
    const page = history();
    page.conversation.active_turn = { turn_id: "running", started_at: start, activities: [] };
    page.approvals = [
      {
        id: "req-1",
        service_slug: "api-github",
        service_name: "GitHub",
        summary: "GET /user",
        approval_mode: "per_request",
        agent_key_prefix: "nyxid_ag_1234",
        created_at: afterReset,
        expires_at: "2026-09-17T00:05:00Z",
      },
    ];
    return json(page);
  };
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const session = transport.session(id);
  const card = session.messages.at(-1);
  expect(card?.id).toBe("nyxagent-approval:req-1");
  expect(card?.role).toBe("system");
  expect(card?.content).toBe("GET /user");
  expect(transport.getHistory(id)?.approvals[0]?.approval_mode).toBe("per_request");
});
