import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NyxAgentTransport, CONTEXT_RESET_NOTICE, storedError, contextResetCopy } from "./nyxagent-transport";
import { transitionAssistantIdentity } from "./identity";
import { assistantChatSurface } from "./conversation-ids";
import type { NyxAgentConversation, NyxAgentHistory } from "@/schemas/assistant-nyxagent";
import { nyxAgentMessageSchema } from "@/schemas/assistant-nyxagent";
import { isAdmittedSendError } from "./send-admission";

const id = `nyxa-${"1".repeat(32)}`;
const other = `nyxa-${"2".repeat(32)}`;
const start = "2026-09-17T00:00:00Z";
const reset = "2026-09-17T00:00:01Z";
const afterReset = "2026-09-17T00:00:02Z";

it.each([
  ["turn_lost", "interrupted"], ["assistant_unavailable", "temporarily unavailable"],
  ["first_byte_timeout", "did not start"], ["invalid_stream", "ended unexpectedly"],
  ["output_too_large", "response limit"], ["nyxid_error", "authorize"],
  ["stale_response", "context changed"], ["context_unavailable", "could not be restored"],
  ["some_upstream_code", "could not complete"],
])("maps %s to actionable copy", (code, text) => {
  expect(storedError(code)).toContain(text);
});

it("maps an unknown raw upstream code to specific recoverable copy", () => {
  expect(storedError("provider_rate_limited")).toContain("could not complete");
});

it("keeps a persisted diagnostic detail visible after reload", () => {
  const parsed = nyxAgentMessageSchema.parse({
    id: "failed", seq: 2, turn_id: "turn-1", role: "assistant", text: "",
    status: "failed", error_code: "turn_lost", created_at: start,
    activities: [], attachments: [],
    error_detail: { stage: "lease", http_status: 502, upstream_request_id: "req-1", response_id: "resp-1", continuations: 2, turn_id: "turn-1", occurred_at: start },
  });
  expect(parsed.error_detail?.turn_id).toBe("turn-1");
  expect(parsed.error_detail?.upstream_request_id).toBe("req-1");
});

it.each([["turn_failed", CONTEXT_RESET_NOTICE], ["stopped", "The turn was stopped, so the assistant continues in a fresh context with a recap. Check what the stopped turn finished before rerunning it."], ["credential_replaced", "The assistant's key was replaced, so it continues in a fresh context with a recap of this chat."], ["user_reset", "You reset the working context."]])("explains reset reason %s", (reason, text) => {
  expect(contextResetCopy(reason)).toBe(text);
  expect(contextResetCopy(reason)).not.toContain("decided cards were preserved");
});

function conversation(key = id): NyxAgentConversation {
  return {
    id: key,
    title: "Question",
    model: "nyxagent/chat",
    role: "orchestrator",
    agent: null,
    pending_events: 0,
    channel: null,
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
        attachments: [],
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
        attachments: [],
      },
    ],
    before_seq: null,
    acknowledgements: [],
    approvals: [],
    waiting: [],
  };
}

const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });

function stream(): Response {
  const events = [
    {
      event: "turn.status",
      conversation_id: id,
      turn_id: "turn",
      status: "running",
    },
    { event: "message.started", message_id: "a", role: "assistant" },
    {
      event: "block.started",
      block_id: "b",
      message_id: "a",
      index: 0,
      block: { type: "text", block_id: "b", text: "" },
    },
    {
      event: "turn.notice",
      code: "context_reset",
      message: "ignore upstream text",
    },
    { event: "block.delta", block_id: "b", text: "Answer 界" },
    {
      event: "block.completed",
      block_id: "b",
      block: { type: "text", block_id: "b", text: "Answer 界" },
    },
    { event: "message.completed", message_id: "a" },
    {
      event: "turn.completed",
      turn_id: "turn",
      status: "completed",
      error: null,
    },
  ];
  const bytes = new TextEncoder().encode(
    events
      .map(
        (event, i) =>
          `data: ${JSON.stringify({ ...event, cursor: i + 1 })}\r\n\r\n`,
      )
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
  it("keeps stopped replies complete with a cancellation code", async () => {
    globalThis.__nyxidAssistantHttpMock = () =>
      json({
        conversation: conversation(),
        messages: [
          {
            id: "stopped",
            seq: 1,
            turn_id: "t",
            role: "assistant",
            text: "Partial",
            status: "failed",
            error_code: "cancelled",
            created_at: start,
            activities: [],
            attachments: [],
          },
        ],
        before_seq: null,
        acknowledgements: [],
        approvals: [],
        waiting: [],
      });
    const transport = new NyxAgentTransport();
    await transport.history(id);
    const message = transport.session(id).messages[0];
    expect(message).toBeDefined();
    if (!message) throw new Error("stopped reply was not loaded");
    expect(message.status).toBe("complete");
    expect(message.errorCode).toBe("cancelled");
    expect(message.error).toBeUndefined();
  });
  it("marks cancelled transport replies ineligible for feedback", async () => {
    globalThis.__nyxidAssistantHttpMock = () =>
      json({
        conversation: conversation(),
        messages: [
          {
            id: "stopped",
            seq: 1,
            turn_id: "t",
            role: "assistant",
            text: "Partial",
            status: "failed",
            error_code: "cancelled",
            created_at: start,
            activities: [],
            attachments: [],
          },
        ],
        before_seq: null,
        acknowledgements: [],
        approvals: [],
        waiting: [],
      });
    const transport = new NyxAgentTransport();
    await transport.history(id);
    expect(transport.session(id).messages[0]?.errorCode).toBe("cancelled");
  });
  it("retries the clicked failed reply without re-sending its bound uploads", async () => {
    const message = (overrides: Record<string, unknown>) => ({
      status: "completed",
      error_code: null,
      created_at: start,
      activities: [],
      attachments: [],
      ...overrides,
    });
    const transcript = {
      conversation: conversation(),
      messages: [
        message({
          id: "u1", seq: 1, turn_id: "t1", role: "user", text: "Send the brief",
          attachments: [{ id: "att-1", label: "brief.txt", content_type: "text/plain", size: 9 }],
        }),
        message({ id: "f1", seq: 2, turn_id: "t1", role: "assistant", text: "", status: "failed", error_code: "idle_timeout" }),
        message({ id: "u2", seq: 3, turn_id: "t2", role: "user", text: "Newer request" }),
        message({ id: "f2", seq: 4, turn_id: "t2", role: "assistant", text: "", status: "failed", error_code: "turn_lost" }),
      ],
      before_seq: null,
      acknowledgements: [],
      approvals: [],
      waiting: [],
    };
    const bodies: Record<string, unknown>[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (endpoint.endsWith("/turns")) {
        bodies.push(JSON.parse(String(init.body)));
        return stream();
      }
      return json(transcript);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    await transport.retry(id, "f1", () => undefined);
    expect(bodies).toEqual([{ conversation_id: id, text: "Send the brief", retry_of: "t1" }]);
    await expect(transport.retry(id, "u2", () => undefined)).rejects.toThrow(
      "This reply can no longer be retried",
    );
  });
  it("retains a linked card during internal refresh and pagination, then clears its scope", async () => {
    const card = "12345678-1234-4123-8123-1234567890ab";
    const requests: string[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      requests.push(endpoint);
      const page = history();
      if (endpoint.includes(`card=${card}`)) {
        page.acknowledgements = [{
          id: card, kind: "action", status: "expired", summary: "Review old draft",
          decider: "user", decided_by: null, reason: null, service_slug: null,
          service_name: null, tool_name: null, created_at: start,
          decided_at: null, expires_at: start,
        }];
      }
      return json(page);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id, undefined, card);
    await transport.history(id);
    const older = await transport.history(id, 2);
    expect(older.acknowledgements.map((row) => row.id)).toEqual([card]);
    expect(requests.slice(0, 3).every((endpoint) => endpoint.includes(`card=${card}`))).toBe(true);
    await transport.history(id, undefined, null);
    await transport.history(id);
    expect(requests.slice(3).every((endpoint) => !endpoint.includes("card="))).toBe(true);
    await transport.history(id, undefined, card);
    transport.clear();
    await transport.history(id);
    expect(requests.at(-1)).not.toContain("card=");
  });

  it("rejects a delayed refresh after the selected review card changes", async () => {
    let finishOld!: (response: Response) => void;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) =>
      endpoint.includes("card=old")
        ? new Promise<Response>((resolve) => { finishOld = resolve; })
        : json(history());
    const transport = new NyxAgentTransport();
    const old = transport.history(id, undefined, "old");
    const rejected = expect(old).rejects.toMatchObject({ name: "AbortError" });
    await transport.history(id, undefined, "new");
    finishOld(json(history()));
    await rejected;
    await transport.history(id);
  });

  it.each([false, true])("loads a linked card with older-replica fallback=%s", async (oldReplica) => {
    const card = "12345678-1234-4123-8123-1234567890ab";
    const requests: string[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      requests.push(endpoint);
      if (oldReplica && endpoint.includes("&card=")) return json({ message: "Unknown field card" }, 400);
      return json(history());
    };
    const transport = new NyxAgentTransport();
    const page = await transport.history(id, undefined, card);
    expect(page.conversation.id).toBe(id);
    expect(requests[0]).toContain(`&card=${card}`);
    expect(requests).toHaveLength(oldReplica ? 2 : 1);
    if (oldReplica) expect(requests[1]).not.toContain("&card=");
  });

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
        sawLiveNotice ||= transport
          .session(id)
          .messages.some((m) => m.role === "system");
      }
    });
    transport.setModel("nyxagent/research");
    await transport.send(undefined, "Question", adopted);
    expect(adopted).toHaveBeenCalledExactlyOnceWith(id);
    expect(sawLiveNotice).toBe(true);
    const messages = transport.session(id).messages;
    expect(messages.map((m) => m.role)).toEqual([
      "user",
      "system",
      "assistant",
    ]);
    expect(messages[1]?.content).toBe(CONTEXT_RESET_NOTICE);
    expect(messages.at(-1)?.content).toBe("Answer 界");
    expect(transport.isRunning(id)).toBe(false);
    expect(JSON.parse(String(requests[0]?.body))).toEqual({
      text: "Question",
      model: "nyxagent/research",
    });
    expect(requests.filter((r) => r.endpoint.endsWith("/turns"))).toHaveLength(
      1,
    );
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
    expect(transport.getHistory(id)?.messages.map((m) => m.seq)).toEqual([
      1, 2,
    ]);
    expect(transport.getHistory(id)?.before_seq).toBeNull();
  });

  it("marks messages the user wrote in a chat app with its platform", async () => {
    globalThis.__nyxidAssistantHttpMock = () => {
      const page = history();
      page.messages = [
        { ...page.messages[0]!, via: "telegram" },
        page.messages[1]!,
      ];
      return json(page);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    expect(
      transport.session(id).messages.map((message) => message.via ?? null),
    ).toEqual(["telegram", null]);
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
    await expect(transport.send(id, "Question", vi.fn())).rejects.toMatchObject(
      {
      status: 409,
      message: "A turn is already active",
      },
    );
    expect(transport.isRunning(id)).toBe(false);
  });

  it("reloads an active turn without resending and uses the owner-only stop endpoint", async () => {
    const requests: string[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      requests.push(`${init.method} ${endpoint}`);
      if (endpoint.endsWith("/stop"))
        return new Response(null, { status: 204 });
      const page = history();
      page.messages.pop();
      page.conversation.active_turn = {
        turn_id: "running",
        started_at: start,
        activities: [
          {
            id: "t1",
            label: "nyx__search_tools",
            status: "completed",
            started_at: start,
            ended_at: reset,
          },
          {
            id: "t2",
            label: "github__list_issues",
            status: "running",
            started_at: reset,
            ended_at: null,
          },
        ],
        attachments: [
          {
            id: "img-1",
            content_type: "image/jpeg",
            size: 10,
            label: "lobby-camera__snapshot",
            origin: "tool",
            image_input: "unavailable",
          },
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
      {
        id: "t1",
        name: "nyx__search_tools",
        status: "done",
        startedAt: Date.parse(start),
        finishedAt: Date.parse(reset),
      },
      {
        id: "t2",
        name: "github__list_issues",
        status: "running",
        startedAt: Date.parse(reset),
        finishedAt: undefined,
      },
    ]);
    // Images a tool returned during the live turn show on the streaming placeholder.
    expect(session.messages.at(-1)?.images).toEqual([
      {
        id: "img-1",
        endpoint: `/assistant/nyxagent/conversations/${id}/attachments/img-1`,
        contentType: "image/jpeg",
        label: "lobby-camera__snapshot",
        origin: "tool",
        imageInput: "unavailable",
      },
    ]);
    await transport.stop(id);
    expect(requests).toHaveLength(2);
    expect(requests[1]).toBe(
      `POST /assistant/nyxagent/conversations/${id}/stop`,
    );
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
      if (init.method === "PATCH")
        return json({ ...conversation(), title: "Renamed" });
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
    await expect(
      new NyxAgentTransport().send(undefined, "Question", vi.fn()),
    ).rejects.toThrow();
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
              controller.enqueue(
                new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
              );
            },
            cancel: cancelled,
          }),
        );
      }
      const page = history();
      page.conversation.active_turn = {
        turn_id: "turn",
        started_at: start,
        activities: [],
        attachments: [],
      };
      return endpoint.includes(`/conversations/${id}`)
        ? json(page)
        : json({ conversations: [page.conversation], next_cursor: null });
    };
    try {
      const transport = new NyxAgentTransport();
      const pending = transport.send(id, "Question", vi.fn());
      const rejected = expect(pending).rejects.toThrow(
        "connection was interrupted",
      );
      await vi.advanceTimersByTimeAsync(135_001);
      await rejected;
      expect(cancelled).toHaveBeenCalledOnce();
      expect(transport.isRunning(id)).toBe(true);
      expect(requests.filter((path) => path.endsWith("/turns"))).toHaveLength(
        1,
      );
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
    page.messages[1] = {
      ...page.messages[1]!,
      status: "failed",
      error_code: "outcome_unknown",
    };
    globalThis.__nyxidAssistantHttpMock = () => json(page);
    const transport = new NyxAgentTransport();
    await transport.history(id);
    expect(transport.session(id).messages.at(-1)?.error).toContain(
      "may have taken effect",
    );
  });

  it("detects repeated index cursors", async () => {
    globalThis.__nyxidAssistantHttpMock = () =>
      json({ conversations: [], next_cursor: "same" });
    await expect(new NyxAgentTransport().list()).rejects.toThrow(
      "Invalid assistant pagination",
    );
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
    decider: "user" as const,
    decided_by: null,
    reason: null,
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
  expect(transport.getHistory(id)?.conversation.pending_acknowledgements).toBe(
    0,
  );
  expect(
    transport.session(id).messages.filter((m) => m.role === "user"),
  ).toHaveLength(1);
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

it("never sends an access mode: every chat runs with Full access", async () => {
  const transport = new NyxAgentTransport();
  const requests: { method: string; endpoint: string; body: unknown }[] = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const body = init.body ? (JSON.parse(String(init.body)) as unknown) : null;
    requests.push({ method: init.method ?? "GET", endpoint, body });
    if (endpoint.endsWith("/turns")) return stream();
    if (endpoint.includes("/conversations?")) {
      return json({ conversations: [conversation()], next_cursor: null });
    }
    // Older servers still echo the retired field; it is ignored.
    return json({
      ...history(),
      conversation: { ...conversation(), access_mode: "full" },
    });
  };
  await transport.send(undefined, "Full draft", vi.fn());
  expect(requests.find((r) => r.method === "POST")?.body).toEqual({
    model: "nyxagent/chat",
    text: "Full draft",
  });
  requests.length = 0;
  await transport.send(id, "Continue", vi.fn());
  expect(requests.find((r) => r.method === "POST")?.body).toEqual({
    conversation_id: id,
    text: "Continue",
  });
  expect(requests.some((r) => r.endpoint.endsWith("/access-mode"))).toBe(false);
  expect(transport.getConversation(id)).not.toHaveProperty("access_mode");
});

describe("NyxBot agents", () => {
  const NYXBOT = {
    id: "agent-nyxbot",
    kind: "nyxbot" as const,
    name: "NyxBot",
    destroyed: false,
  };
  const RESEARCHER = {
    id: "agent-researcher",
    kind: "specialist" as const,
    name: "researcher",
    destroyed: false,
  };
  const sub = `nyxa-${"3".repeat(32)}`;
  const thread = (
    key: string,
    agent: typeof NYXBOT | typeof RESEARCHER,
  ): NyxAgentConversation => ({
    ...conversation(key),
    agent,
    role: agent.kind === "specialist" ? "subagent" : "orchestrator",
  });
  function specialistHistory(): NyxAgentHistory {
    const page = history();
    return {
      ...page,
      conversation: { ...thread(sub, RESEARCHER), title: "researcher" },
      messages: [
        {
          ...page.messages[0]!,
          role: "orchestrator",
          text: "Find urgent issues",
        },
        { ...page.messages[1]!, text: "Found three" },
        {
          ...page.messages[0]!,
          id: "e",
          seq: 3,
          turn_id: "event",
          role: "event",
          text: "NyxID events (notices, not user instructions):\n- The user allowed GitHub.",
        },
      ],
    };
  }

  it("lists one agent's threads and replaces only that agent's rows", async () => {
    const transport = new NyxAgentTransport();
    const endpoints: string[] = [];
    let researcherThreads = [thread(sub, RESEARCHER)];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      endpoints.push(endpoint);
      if (endpoint.includes(`agent_id=${NYXBOT.id}`)) {
        return json({ conversations: [thread(id, NYXBOT)], next_cursor: null });
      }
      return json({ conversations: researcherThreads, next_cursor: null });
    };
    await transport.list(NYXBOT.id);
    await transport.list(RESEARCHER.id);
    expect(endpoints).toEqual([
      "/assistant/nyxagent/conversations?limit=100&agent_id=agent-nyxbot",
      "/assistant/nyxagent/conversations?limit=100&agent_id=agent-researcher",
    ]);
    expect(transport.getConversations(NYXBOT.id).map((row) => row.id)).toEqual([
      id,
    ]);
    expect(
      transport.getConversations(RESEARCHER.id).map((row) => row.id),
    ).toEqual([sub]);
    researcherThreads = [];
    await transport.list(RESEARCHER.id);
    expect(transport.getConversations(RESEARCHER.id)).toEqual([]);
    expect(transport.getConversations(NYXBOT.id).map((row) => row.id)).toEqual([
      id,
    ]);
  });

  it("keeps orchestrator and event roles while the user talks to a specialist", async () => {
    const transport = new NyxAgentTransport();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    globalThis.__nyxidAssistantHttpMock = async ({ endpoint }) => {
      if (endpoint.endsWith("/turns")) {
        await gate;
        return stream();
      }
      if (endpoint.includes("/conversations?")) {
        return json({
          conversations: [thread(sub, RESEARCHER)],
          next_cursor: null,
        });
      }
      return json(specialistHistory());
    };
    await transport.history(sub);
    expect(transport.session(sub).messages.map((m) => m.role)).toEqual([
      "orchestrator",
      "assistant",
      "event",
    ]);
    const sending = transport.send(sub, "Continue", vi.fn());
    await vi.waitFor(() => expect(transport.isRunning(sub)).toBe(true));
    expect(
      transport
        .session(sub)
        .messages.slice(0, 3)
        .map((m) => m.role),
    ).toEqual(["orchestrator", "assistant", "event"]);
    release();
    await sending.catch(() => undefined);
  });

  it("starts a new thread with the chosen agent and files it under that agent at once", async () => {
    const transport = new NyxAgentTransport();
    const bodies: unknown[] = [];
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    globalThis.__nyxidAssistantHttpMock = async ({ endpoint, init }) => {
      if (endpoint.endsWith("/turns")) {
        bodies.push(JSON.parse(String(init.body)));
        await gate;
        return stream();
      }
      if (endpoint.includes("/conversations?")) {
        return json({
          conversations: [thread(id, RESEARCHER)],
          next_cursor: null,
        });
      }
      return json({ ...history(), conversation: thread(id, RESEARCHER) });
    };
    const sending = transport.send(undefined, "Hello", vi.fn(), undefined, {
      agent: RESEARCHER,
    });
    await vi.waitFor(() => expect(bodies).toHaveLength(1));
    expect(bodies[0]).toEqual({
      model: "nyxagent/chat",
      agent_id: RESEARCHER.id,
      text: "Hello",
    });
    release();
    await sending;
    expect(
      transport.getConversations(RESEARCHER.id).map((row) => row.id),
    ).toEqual([id]);
  });

  it("keeps the agent and pending counts when a rename returns a bare row", async () => {
    const transport = new NyxAgentTransport();
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (init.method === "PATCH") {
        return json({ ...conversation(), title: "Renamed", agent: null });
      }
      if (endpoint.includes("/conversations?")) {
        return json({
          conversations: [
            { ...thread(id, NYXBOT), pending_acknowledgements: 2 },
          ],
          next_cursor: null,
        });
      }
      return json(history());
    };
    await transport.list(NYXBOT.id);
    await transport.rename(id, "Renamed");
    expect(transport.getConversations(NYXBOT.id)[0]).toMatchObject({
      title: "Renamed",
      pending_acknowledgements: 2,
      agent: NYXBOT,
    });
  });

  it("deletes one thread only, and forgets a deleted agent's threads", async () => {
    const transport = new NyxAgentTransport();
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (init.method === "DELETE") return new Response(null, { status: 204 });
      if (endpoint.includes(`agent_id=${NYXBOT.id}`)) {
        return json({
          conversations: [thread(id, NYXBOT), thread(other, NYXBOT)],
          next_cursor: null,
        });
      }
      if (endpoint.includes("/conversations?")) {
        return json({
          conversations: [thread(sub, RESEARCHER)],
          next_cursor: null,
        });
      }
      return json(specialistHistory());
    };
    await transport.list(NYXBOT.id);
    await transport.list(RESEARCHER.id);
    await transport.history(sub);
    await transport.delete(id);
    expect(transport.getConversations(NYXBOT.id).map((row) => row.id)).toEqual([
      other,
    ]);
    transport.forgetAgent(RESEARCHER.id);
    expect(transport.getConversation(sub)).toBeUndefined();
    expect(transport.getConversations(NYXBOT.id).map((row) => row.id)).toEqual([
      other,
    ]);
  });
});

it("retains a settled reply's tool activity as done tool calls", async () => {
  globalThis.__nyxidAssistantHttpMock = () => {
    const page = history();
    page.messages[1] = {
      ...page.messages[1]!,
      activities: [
        {
          id: "t1",
          label: "nyxid__list_agent_keys",
          status: "error",
          started_at: start,
          ended_at: reset,
        },
      ],
      attachments: [
        {
          id: "img-2",
          content_type: "image/png",
          size: 4,
          label: "frigate__latest",
        },
      ],
    };
    return json(page);
  };
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const session = transport.session(id);
  expect(session.messages[0]?.toolCalls).toBeUndefined();
  expect(session.messages[0]?.images).toBeUndefined();
  expect(session.messages[1]?.images).toEqual([
    {
      id: "img-2",
      endpoint: `/assistant/nyxagent/conversations/${id}/attachments/img-2`,
      contentType: "image/png",
      label: "frigate__latest",
    },
  ]);
  expect(session.messages[1]?.toolCalls).toEqual([
    {
      id: "t1",
      name: "nyxid__list_agent_keys",
      status: "error",
      startedAt: Date.parse(start),
      finishedAt: Date.parse(reset),
    },
  ]);
});

it("shows pending proxy approvals raised by the chat key as cards at the tail", async () => {
  globalThis.__nyxidAssistantHttpMock = () => {
    const page = history();
    page.conversation.active_turn = {
      turn_id: "running",
      started_at: start,
      activities: [],
      attachments: [],
    };
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
  expect(transport.getHistory(id)?.approvals[0]?.approval_mode).toBe(
    "per_request",
  );
});

it("optimistically renames during a turn and rolls back on failure without losing activity", async () => {
  let finish!: (response: Response) => void;
  const activeRow = {
    ...conversation(),
    active_turn: {
      turn_id: "running",
      started_at: start,
      stop_requested: false,
      continuations: 0,
      activities: [],
      attachments: [],
    },
  };
  globalThis.__nyxidAssistantHttpMock = ({ init }) =>
    init.method === "PATCH"
      ? new Promise<Response>((resolve) => {
          finish = resolve;
        })
    : json({ ...history(), conversation: activeRow });
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const pending = transport.rename(id, "My chosen title");
  expect(transport.getConversations()[0]!.title).toBe("My chosen title");
  expect(transport.session(id).title).toBe("My chosen title");
  await transport.history(id); // stale server metadata cannot overwrite the optimistic title
  expect(transport.session(id).title).toBe("My chosen title");
  finish(json({ error: { message: "Try again" } }, 500));
  await expect(pending).rejects.toThrow();
  expect(transport.session(id).title).toBe("Question");
  expect(transport.isRunning(id)).toBe(true);
});

describe("steering transport", () => {
  it("posts the exact stable request and refreshes history after response mismatch", async () => {
    const seen: { endpoint: string; body?: BodyInit | null }[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      seen.push({ endpoint, body: init.body });
      if (endpoint.endsWith("/steer"))
        return json(
          { error: "response_mismatch", message: "Refresh turn" },
          409,
        );
      return json(history());
    };
    const transport = new NyxAgentTransport();
    await expect(
      transport.steer(id, "Guidance", "turn", "stable-request"),
    ).rejects.toThrow("Refresh turn");
    expect(JSON.parse(String(seen[0]?.body))).toEqual({
      text: "Guidance",
      turn_id: "turn",
      clientRequestId: "stable-request",
    });
    expect(seen).toHaveLength(2);
    expect(transport.getHistory(id)?.messages).toHaveLength(2);
  });

  it("merges a steer once during a live stream and preserves its label during a later turn", async () => {
    const page = history();
    page.messages[1] = { ...page.messages[1]!, status: "failed", error_code: "cancelled" };
    const receipt = {
      client_request_id: "guidance",
      outcome: "applied" as const,
      code: null,
    };
    page.messages.splice(1, 0, {
      ...page.messages[0]!,
      id: "steer",
      seq: 2,
      text: "Direction",
      steering: receipt,
    });
    page.messages[2] = { ...page.messages[2]!, seq: 3 };
    let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint.endsWith("/turns"))
        return new Response(
          new ReadableStream({
            start(c) {
              controller = c;
            },
          }),
          { headers: { "content-type": "text/event-stream" } },
        );
      return json(page);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    const sending = transport.send(id, "Next question", vi.fn());
    await vi.waitFor(() => expect(controller).toBeDefined());
    const emit = (event: object) =>
      controller!.enqueue(
        new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
      );
    emit({
      cursor: 1,
      event: "turn.status",
      conversation_id: id,
      turn_id: "next",
      status: "running",
    });
    await vi.waitFor(() => expect(transport.getActiveTurnId(id)).toBe("next"));
    page.conversation.active_turn = {
      turn_id: "next",
      started_at: start,
      steering_allowed: true,
      response_id: "resp_next",
      stop_requested: false,
      activities: [],
      attachments: [],
    };
    page.messages.push({
      ...page.messages[0]!,
      id: "new-steer",
      seq: 4,
      turn_id: "next",
      text: "New direction",
      steering: { ...receipt, client_request_id: "new" },
    });
    await transport.history(id);
    await transport.history(id);
    const session = transport.session(id);
    expect(session.messages.filter((m) => m.steering)).toHaveLength(2);
    expect(session.messages.find((message) => message.id === page.messages[2]?.id)?.errorCode).toBe("cancelled");
    expect(session.messages.find((m) => m.id === "steer")?.steering).toEqual(
      receipt,
    );
    expect(session.expectedTurnCount).toBe(2);
    expect(transport.getConversation(id)?.active_turn?.response_id).toBe(
      "resp_next",
    );
    page.conversation.active_turn = null;
    emit({
      cursor: 2,
      event: "turn.completed",
      turn_id: "next",
      status: "completed",
      error: null,
    });
    controller!.close();
    await sending;
  });
});

function sseBytes(...events: Record<string, unknown>[]): Uint8Array {
  return new TextEncoder().encode(
    events.map((event, i) => `data: ${JSON.stringify({ ...event, cursor: i + 1 })}\n\n`).join(""),
  );
}

function chunked(...parts: Uint8Array[]): Response {
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const part of parts) controller.enqueue(part);
        controller.close();
      },
    }),
    { headers: { "content-type": "text/event-stream" } },
  );
}

const runningStatus = { event: "turn.status", conversation_id: id, turn_id: "turn", status: "running" };

describe("send admission", () => {
  async function sendFailure(turns: () => Response | Promise<Response>): Promise<unknown> {
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint.endsWith("/turns")) return turns();
      return endpoint.includes(`/conversations/${id}`)
        ? json(history())
        : json({ conversations: [], next_cursor: null });
    };
    return new NyxAgentTransport()
      .send(id, "Question", vi.fn())
      .then(() => "resolved", (error: unknown) => error);
  }

  it.each([
    [
      "the 12 MiB response limit",
      () => chunked(sseBytes(runningStatus), new Uint8Array(12 * 1024 * 1024 + 1)),
      "message",
      "Assistant response exceeded its limit.",
    ],
    ["an SSE payload that is not JSON", () => chunked(new TextEncoder().encode("data: broken\n\n")), "name", "SyntaxError"],
    ["a body that is not UTF-8", () => chunked(new Uint8Array([0xff, 0xfe, 0x0a])), "name", "TypeError"],
    [
      "a turn for another conversation",
      () => chunked(sseBytes({ ...runningStatus, conversation_id: other })),
      "message",
      "Assistant conversation mismatch.",
    ],
    [
      "a stream that ends without a terminal event",
      () => chunked(sseBytes(runningStatus)),
      "message",
      "The connection was interrupted. The assistant may still be working.",
    ],
    ["a 2xx without a body", () => new Response(null), "message", "The assistant returned no stream."],
  ] as const)("marks %s after a 2xx as admitted", async (_label, turns, field, expected) => {
    const error = await sendFailure(turns);
    expect(error).toBeInstanceOf(Error);
    expect((error as Error)[field]).toBe(expected);
    expect(isAdmittedSendError(error)).toBe(true);
  });

  it("marks the 135 second read deadline after a 2xx as admitted", async () => {
    vi.useFakeTimers();
    try {
      const pending = sendFailure(
        () => new Response(new ReadableStream({ start(controller) { controller.enqueue(sseBytes(runningStatus)); } })),
      );
      await vi.advanceTimersByTimeAsync(135_001);
      const error = await pending;
      expect((error as Error).message).toBe("The connection was interrupted. The assistant may still be working.");
      expect(isAdmittedSendError(error)).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not mark a refused POST as admitted", async () => {
    const error = await sendFailure(() =>
      json({ error: "turn_active", error_code: 12100, message: "A turn is already active" }, 409),
    );
    expect(error).toMatchObject({ status: 409, message: "A turn is already active" });
    expect(isAdmittedSendError(error)).toBe(false);
  });

  it("does not mark a network failure before any response as admitted", async () => {
    const error = await sendFailure(() => {
      throw new TypeError("Failed to fetch");
    });
    expect((error as Error).message).toBe("Failed to fetch");
    expect(isAdmittedSendError(error)).toBe(false);
  });
});

describe("sent images", () => {
  const photo = { id: "img-1", content_type: "image/png" as const, size: 4, label: "photo.png", origin: "user_upload" as const };
  const photoImage = {
    id: "img-1",
    endpoint: `/assistant/nyxagent/conversations/${id}/attachments/img-1`,
    contentType: "image/png",
    label: "photo.png",
    origin: "user_upload",
  };

  it("shows them on the optimistic user message and exactly once after the history reload", async () => {
    const page = history();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    const bodies: unknown[] = [];
    globalThis.__nyxidAssistantHttpMock = async ({ endpoint, init }) => {
      if (endpoint.endsWith("/turns")) {
        bodies.push(JSON.parse(String(init.body)));
        // Admission stores the user message with its bound upload.
        page.messages.push({
          ...page.messages[0]!, id: "u2", seq: 3, turn_id: "turn-2", text: "Look", created_at: afterReset, attachments: [photo],
        });
        await gate;
        page.messages.push({
          ...page.messages[1]!, id: "a2", seq: 4, turn_id: "turn-2", text: "A photo", created_at: afterReset,
        });
        return chunked(sseBytes(
          { ...runningStatus, turn_id: "turn-2" },
          { event: "message.started", message_id: "a2", role: "assistant" },
          { event: "block.started", block_id: "b2", message_id: "a2", index: 0, block: { type: "text", block_id: "b2", text: "" } },
          { event: "block.delta", block_id: "b2", text: "A photo" },
          { event: "block.completed", block_id: "b2", block: { type: "text", block_id: "b2", text: "A photo" } },
          { event: "message.completed", message_id: "a2" },
          { event: "turn.completed", turn_id: "turn-2", status: "completed", error: null },
        ));
      }
      return endpoint.includes(`/conversations/${id}`)
        ? json(page)
        : json({ conversations: [page.conversation], next_cursor: null });
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    const imageSets = () =>
      transport.session(id).messages.filter((message) => message.images?.length);
    let most = 0;
    transport.subscribe(() => {
      most = Math.max(most, imageSets().length);
    });
    const sending = transport.send(id, "Look", vi.fn(), undefined, {
      attachmentIds: [photo.id],
      attachments: [photo],
    });
    await vi.waitFor(() => expect(bodies).toHaveLength(1));
    expect(bodies[0]).toEqual({ conversation_id: id, text: "Look", attachment_ids: ["img-1"] });
    const optimistic = imageSets();
    expect(optimistic).toHaveLength(1);
    expect(optimistic[0]).toMatchObject({ role: "user", content: "Look", images: [photoImage] });
    // A poll that lands mid-turn already holds the stored message; it must not add a second copy.
    await transport.history(id);
    expect(imageSets()).toHaveLength(1);
    release();
    await sending;
    const settled = transport.session(id).messages;
    expect(settled.map((message) => message.id)).toEqual(["u", "a", "u2", "a2"]);
    expect(imageSets()).toEqual([expect.objectContaining({ id: "u2", images: [photoImage] })]);
    expect(most).toBe(1);
  });
});
