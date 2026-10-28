import { createElement } from "react";
import { capture } from "@/lib/telemetry";

vi.mock("@/lib/telemetry", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/lib/telemetry")>(),
  capture: vi.fn(),
}));
import { render, screen } from "@testing-library/react";
import { ChatMessageList } from "@/components/assistant/chat-message";
import { nyxAgentHistorySchema } from "@/schemas/assistant-nyxagent";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NyxAgentTransport, CONTEXT_RESET_NOTICE, storedError, contextResetCopy } from "./nyxagent-transport";
import { transitionAssistantIdentity } from "./identity";
import { assistantChatSurface } from "./conversation-ids";
import type { NyxAgentConversation, NyxAgentHistory } from "@/schemas/assistant-nyxagent";
import { nyxAgentMessageSchema } from "@/schemas/assistant-nyxagent";
import { isAdmittedSendError } from "./send-admission";
import { applyDirectTurnEvent, type DirectTurnState } from "./direct-transport";

const id = `nyxa-${"1".repeat(32)}`;
const other = `nyxa-${"2".repeat(32)}`;
const start = "2026-09-17T00:00:00Z";
const reset = "2026-09-17T00:00:01Z";
const afterReset = "2026-09-17T00:00:02Z";

it.each([
  ["turn_lost", "interrupted"], ["assistant_unavailable", "could not complete"],
  ["guest_model_route_required", "isn't available to guests yet"],
  ["internal_error", "NyxBot hit an internal error on this turn. Check which actions completed before retrying."],
  ["first_byte_timeout", "did not start"], ["invalid_stream", "ended unexpectedly"],
  ["output_too_large", "response limit"], ["nyxid_error", "authorize"],
  ["stale_response", "context changed"], ["context_unavailable", "could not be restored"],
  ["some_upstream_code", "could not complete"], ["queue_timeout", "was not started"],
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
    cma_operations: [],
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
  vi.mocked(capture).mockReset();
  transitionAssistantIdentity(null);
  transitionAssistantIdentity("owner");
});

afterEach(() => {
  globalThis.__nyxidAssistantHttpMock = undefined;
  transitionAssistantIdentity(null);
  vi.restoreAllMocks();
});

describe("NyxAgent server-backed transport", () => {
  it("keeps CMA status independent of the completed reply, refreshes it, and restores it after reload", async () => {
    const page = history();
    page.cma_operations = [{
      id: "watch-1", turn_id: "turn", kind: "cma_response", operation_id: "resp_agt_test_1",
      agent_id: "agt_test", idempotency_key: "request-1", status: "in_progress", phase: null,
      state: "waiting", reason: null, monitor_name: "NyxBot", created_at: start, checked_at: afterReset,
    }];
    globalThis.__nyxidAssistantHttpMock = () => json(page);
    const transport = new NyxAgentTransport();
    await transport.history(id);
    expect(transport.session(id).messages.find((message) => message.id === "u")?.cmaOperations).toBeUndefined();
    const reply = transport.session(id).messages.find((message) => message.id === "a");
    expect(reply?.status).toBe("complete");
    expect(reply?.cmaOperations?.[0]?.status).toBe("in_progress");
    await transport.history(id);
    expect(transport.session(id).messages.find((message) => message.id === "a")).toBe(reply);
    // The latest page no longer contains the original reply, but the thread's
    // operation snapshot must still settle its card on the retained older page.
    page.messages = [];
    page.cma_operations[0] = { ...page.cma_operations[0]!, state: "delivered", reason: "completed", status: "completed" };
    await transport.history(id);
    expect(transport.session(id).messages.find((message) => message.id === "a")?.cmaOperations?.[0]?.status).toBe("completed");
    const restored = new NyxAgentTransport();
    page.messages = history().messages;
    await restored.history(id);
    expect(restored.session(id).messages.find((message) => message.id === "a")?.cmaOperations).toEqual(page.cma_operations);
    page.cma_operations = [];
    await transport.history(id);
    expect(transport.session(id).messages.every((message) => !message.cmaOperations)).toBe(true);
    const ordinary = new NyxAgentTransport();
    await ordinary.history(id);
    expect(ordinary.session(id).messages.every((message) => !message.cmaOperations)).toBe(true);
  });
  it("keeps stopped replies visibly stopped with a cancellation code", async () => {
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
    expect(message.status).toBe("stopped");
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
      if (endpoint.endsWith("/messages/f1/recovery")) return json({
        status: "available", reason: "available", text: "Send the brief",
        attachments: ["brief.txt"],
      });
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
      "Failed reply not found",
    );
  });
  it("checks a temporarily unavailable paginated attachment-only retry before sending", async () => {
    const transcript = history();
    transcript.messages = [{
      id: "old-failed", seq: 1, turn_id: "old-turn", role: "assistant", text: "",
      status: "failed", error_code: "turn_lost", created_at: start,
      activities: [], attachments: [],
      recovery: { status: "review", reason: "turn_active", text: "", attachments: ["scan.pdf"] },
    }];
    let checks = 0;
    const bodies: Record<string, unknown>[] = [];
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (endpoint.endsWith("/messages/old-failed/recovery")) {
        checks += 1;
        return checks === 1 ? json({ error: "temporarily unavailable" }, 503)
          : json({ status: "available", reason: "available", text: "", attachments: ["scan.pdf"] });
      }
      if (endpoint.endsWith("/turns")) {
        bodies.push(JSON.parse(String(init.body)));
        return stream();
      }
      return json(transcript);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    expect((await transport.checkRecovery(id, "old-failed")).reason).toBe("status_unavailable");
    expect(transport.session(id).messages.find((message) => message.id === "old-failed")?.recovery?.reason)
      .toBe("status_unavailable");
    expect((await transport.checkRecovery(id, "old-failed")).status).toBe("available");
    await transport.retry(id, "old-failed", () => undefined);
    expect(bodies).toEqual([{ conversation_id: id, text: "", retry_of: "old-turn" }]);
  });
  it("does not turn an admitted retry into an error when final recovery refresh races history", async () => {
    const oldPage = history();
    oldPage.messages = [{ id: "failed", seq: 1, turn_id: "old-turn", role: "assistant",
      text: "", status: "failed", error_code: "turn_lost", created_at: start,
      activities: [], attachments: [], recovery: { status: "available", reason: "available",
        text: "restore", attachments: [] } }];
    const newest = history();
    newest.messages = Array.from({ length: 100 }, (_, index) => ({
      id: `new-${index}`, seq: 101 + index, turn_id: `new-turn-${index}`, role: "user",
      text: "newer", status: "completed", error_code: null, created_at: start,
      activities: [], attachments: [],
    }));
    newest.before_seq = 100;
    let checks = 0;
    let finalStarted: () => void = () => undefined;
    const started = new Promise<void>((resolve) => { finalStarted = resolve; });
    let resolveFinal: (response: Response) => void = () => undefined;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint.endsWith("/messages/failed/recovery")) {
        checks += 1;
        if (checks === 1) return json({ status: "available", reason: "available", text: "restore", attachments: [] });
        return new Promise<Response>((resolve) => { resolveFinal = resolve; finalStarted(); });
      }
      if (endpoint.endsWith("/turns")) return stream();
      return json(endpoint.includes("before_seq=100") ? oldPage : newest);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    await transport.history(id, 100);
    const attempt = transport.retry(id, "failed", () => undefined);
    await started;
    await transport.history(id);
    resolveFinal(json({ status: "available", reason: "available", text: "restore", attachments: [] }));
    await expect(attempt).resolves.toBeUndefined();
    expect(transport.session(id).messages[0]?.recovery?.reason).toBe("already_retried");
  });
  it("reconciles an admission rejection through the clicked reply without resending", async () => {
    const transcript = history();
    transcript.messages = [{ id: "failed", seq: 1, turn_id: "old-turn", role: "assistant",
      text: "", status: "failed", error_code: "turn_lost", created_at: start,
      activities: [], attachments: [], recovery: { status: "available", reason: "available",
        text: "restore", attachments: ["old.txt"] } }];
    let checks = 0;
    let posts = 0;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint.endsWith("/messages/failed/recovery")) {
        checks += 1;
        return json(checks === 1
          ? { status: "available", reason: "available", text: "restore", attachments: ["old.txt"] }
          : { status: "review", reason: "attachment_expired", text: "restore", attachments: ["old.txt"] });
      }
      if (endpoint.endsWith("/turns")) {
        posts += 1;
        return json({ error: "attachment_expired" }, 410);
      }
      return json(transcript);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    await expect(transport.retry(id, "failed", () => undefined)).rejects.toThrow();
    expect(posts).toBe(1);
    expect(checks).toBe(2);
    expect(transport.session(id).messages[0]?.recovery?.reason).toBe("attachment_expired");
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

  it("keeps each row's server time through a reload and an earlier page", async () => {
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      const page = history();
      if (endpoint.includes("before_seq")) {
        page.messages = [page.messages[0]!];
      } else {
        page.messages = [page.messages[1]!];
        page.before_seq = 2;
      }
      return json(page);
    };
    const transport = new NyxAgentTransport();
    await transport.history(id);
    await transport.history(id, 2);
    expect(transport.session(id).messages.map((m) => [m.id, m.timestamp])).toEqual([
      ["u", Date.parse(start)],
      ["a", Date.parse(afterReset)],
    ]);
  });

  it("reuses unchanged rows across reads and identical refreshes, so only changed rows re-render", async () => {
    let page = history();
    globalThis.__nyxidAssistantHttpMock = () => json(page);
    const transport = new NyxAgentTransport();
    await transport.history(id);
    const first = transport.session(id).messages;
    expect(transport.session(id).messages).toEqual(first);
    expect(transport.session(id).messages[0]).toBe(first[0]);
    await transport.history(id);
    const refreshed = transport.session(id).messages;
    expect(refreshed[0]).toBe(first[0]);
    expect(refreshed[1]).toBe(first[1]);
    page = history();
    page.messages[1] = { ...page.messages[1]!, text: "Edited" };
    await transport.history(id);
    const edited = transport.session(id).messages;
    expect(edited[0]).toBe(first[0]);
    expect(edited[1]).not.toBe(first[1]);
    expect(edited[1]!.content).toBe("Edited");
  });

  it("changes only the message that owns a streamed block", () => {
    const settled = {
      id: "u", role: "user" as const, schema_version: 1 as const, created_at: start,
      blocks: [{ type: "text" as const, block_id: "u", text: "Question" }],
    };
    const state: DirectTurnState = {
      lastCursor: 3,
      activeTurn: null,
      messages: [settled, {
        id: "a", role: "assistant", schema_version: 1, created_at: start,
        blocks: [{ type: "text", block_id: "b", text: "An" }],
      }],
    };
    const delta = applyDirectTurnEvent(state, { event: "block.delta", cursor: 4, block_id: "b", text: "swer" });
    expect(delta.messages[0]).toBe(settled);
    expect(delta.messages[1]!.blocks[0]!.text).toBe("Answer");
    const completed = applyDirectTurnEvent(delta, {
      event: "block.completed", cursor: 5, block_id: "b", block: { type: "text", block_id: "b", text: "Answer!" },
    });
    expect(completed.messages[0]).toBe(settled);
    expect(completed.messages[1]!.blocks[0]!.text).toBe("Answer!");
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
      page.messages = page.messages.filter((message) => message.role !== "assistant");
      return endpoint.includes(`/conversations/${id}`)
        ? json(page)
        : json({ conversations: [page.conversation], next_cursor: null });
    };
    try {
      const transport = new NyxAgentTransport();
      const pending = transport.send(id, "Question", vi.fn());
      const rejected = expect(pending).rejects.toThrow(
        "live reply was interrupted",
      );
      await vi.advanceTimersByTimeAsync(135_001);
      await rejected;
      expect(cancelled).toHaveBeenCalledOnce();
      expect(transport.isRunning(id)).toBe(true);
      expect(requests.filter((path) => path.endsWith("/turns"))).toHaveLength(
        1,
      );
      expect(requests.some((path) => path.endsWith("/stop"))).toBe(false);
      expect(capture).toHaveBeenCalledExactlyOnceWith({ name: "nyxbot.error", props: expect.objectContaining({
        error_code: "NYXBOT_STREAM_TIMEOUT", engine: "nyxagent", stage: "stream", http_status: 200,
      }) });
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

  it("keeps the idle-timeout warning uncertainty-aware when restoring a transcript", async () => {
    const page = history();
    page.messages[1] = { ...page.messages[1]!, status: "failed", error_code: "idle_timeout" };
    globalThis.__nyxidAssistantHttpMock = () => json(page);
    const transport = new NyxAgentTransport();
    await transport.history(id);
    expect(transport.session(id).messages.at(-1)?.error).toContain("Check what it already did");
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

it("tracks a failed live stream through polling and clears interruption on settle", async () => {
  let active = true;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    if (endpoint.endsWith("/turns")) return new Response(new ReadableStream({ start(controller) {
      controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify({ event: "turn.status", conversation_id: id, turn_id: "turn", status: "running", cursor: 1 })}\n\n`));
      controller.close();
    } }));
    const page = history();
    if (active) { page.messages.pop(); page.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [], attachments: [] }; }
    return json(page);
  };
  const transport = new NyxAgentTransport();
  await transport.history(id);
  // send refuses a known active turn, so let admission establish it first.
  active = false;
  await transport.history(id);
  active = true;
  await expect(transport.send(id, "Question", vi.fn())).rejects.toThrow(/interrupted/);
  await transport.history(id);
  expect(transport.session(id).messages.find((message) => message.status === "streaming")?.turnStatus?.interrupted).toBe(true);
  active = false;
  await transport.history(id);
  expect(transport.session(id).messages.some((message) => message.turnStatus?.interrupted)).toBe(false);
});

it("restores partial text after reload and ignores older checkpoint polls", async () => {
  const page = history();
  page.messages = page.messages.slice(0, 1);
  page.conversation.active_turn = { turn_id: "running", started_at: start, activities: [], attachments: [],
    reply: { message_id: "reply", block_id: "text", cursor: 10, text: "Already written 界" } };
  globalThis.__nyxidAssistantHttpMock = () => json(page);
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const first = transport.session(id).messages.at(-1)!;
  expect(first).toMatchObject({ content: "Already written 界", status: "streaming", timestamp: Date.parse(start) });
  page.conversation.active_turn.reply = { message_id: "reply", block_id: "text", cursor: 8, text: "Already" };
  await transport.history(id);
  expect(transport.session(id).messages.at(-1)?.content).toBe("Already written 界");
});

it("adopts a new thread from its response header when initial stream events are lost", async () => {
  const page = history();
  page.messages = page.messages.slice(0, 1);
  page.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [], attachments: [],
    reply: { message_id: "reply", block_id: "text", cursor: 4, text: "Recovered from the checkpoint" } };
  let reader!: ReadableStreamDefaultController<Uint8Array>;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? new Response(new ReadableStream({ start(controller) { reader = controller; } }), { headers: { "X-NyxID-Conversation-Id": id } })
    : json(page);
  const adopt = vi.fn();
  const transport = new NyxAgentTransport();
  const sending = transport.send(undefined, "hello", adopt);
  const interrupted = expect(sending).rejects.toThrow("interrupted");
  try {
    await vi.waitFor(() => expect(transport.session(id).messages.at(-1)).toMatchObject({
      content: "Recovered from the checkpoint", status: "streaming",
      turnStatus: { startedAt: start },
    }));
  } finally {
    reader.close();
    await interrupted;
  }
  expect(adopt).toHaveBeenCalledExactlyOnceWith(id);
  expect(transport.session(id).messages.at(-1)).toMatchObject({ content: "Recovered from the checkpoint", status: "streaming" });
});

it("keeps text received after the latest saved checkpoint when the reader disconnects", async () => {
  const page = history();
  page.messages = page.messages.slice(0, 1);
  page.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [], attachments: [] };
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? new Response(new ReadableStream({ start(controller) {
        controller.enqueue(sseBytes(runningStatus,
          { event: "message.started", message_id: "reply", role: "assistant" },
          { event: "block.started", message_id: "reply", block_id: "text", index: 0, block: { type: "text", block_id: "text", text: "" } },
          { event: "block.delta", block_id: "text", text: "Keep these words" }));
        controller.close();
      } })) : json(page);
  const transport = new NyxAgentTransport();
  await expect(transport.send(id, "hello", vi.fn())).rejects.toThrow("interrupted");
  expect(transport.session(id).messages.at(-1)?.content).toBe("Keep these words");
  await transport.history(id);
  expect(transport.session(id).messages.at(-1)?.content).toBe("Keep these words");
});

it("repairs missed text and block events from a checkpoint without duplicating delayed deltas", async () => {
  const page = history();
  page.messages = page.messages.slice(0, 1);
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? new Response(new ReadableStream({ start(value) { controller = value; } })) : json(page);
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const sending = transport.send(id, "hello", vi.fn());
  await vi.waitFor(() => expect(controller).toBeDefined());
  const emit = (event: Record<string, unknown>, cursor: number) => controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify({ ...event, cursor })}\n\n`));
  emit(runningStatus, 1);
  emit({ event: "block.delta", block_id: "text", text: "missed start" }, 4);
  emit({ ...runningStatus, engine_accepted: true }, 8);
  await vi.waitFor(() => expect(transport.getActiveTurnId(id)).toBe("turn"));
  page.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [], attachments: [],
    reply: { message_id: "reply", block_id: "text", cursor: 7, text: "Recovered 界" } };
  await transport.history(id);
  expect(transport.session(id).messages.at(-1)?.content).toBe("Recovered 界");
  emit({ event: "block.delta", block_id: "text", text: "duplicate" }, 6);
  emit({ event: "block.delta", block_id: "text", text: " continues" }, 9);
  await vi.waitFor(() => expect(transport.session(id).messages.at(-1)?.content).toBe("Recovered 界 continues"));
  emit({ event: "turn.completed", turn_id: "turn", status: "completed", error: null }, 10);
  page.conversation.active_turn = null;
  controller.close();
  await sending;
});

it("shows a durable completed reply and releases a stalled stream without resending", async () => {
  const page = history();
  page.messages = page.messages.slice(0, 1);
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  const cancelled = vi.fn();
  const posts = vi.fn();
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    if (!endpoint.endsWith("/turns")) return json(page);
    posts();
    return new Response(new ReadableStream({ start(value) { controller = value; }, cancel: cancelled }));
  };
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const sending = transport.send(id, "hello", vi.fn());
  await vi.waitFor(() => expect(controller).toBeDefined());
  controller.enqueue(sseBytes(runningStatus, { event: "message.started", message_id: "reply", role: "assistant" }));
  await vi.waitFor(() => expect(transport.getActiveTurnId(id)).toBe("turn"));
  page.messages.push({ ...history().messages[1]!, id: "reply", text: "The full durable answer" });
  await transport.history(id);
  await sending;
  expect(transport.session(id).messages.at(-1)).toMatchObject({ content: "The full durable answer", status: "complete" });
  expect(transport.isRunning(id)).toBe(false);
  expect(cancelled).toHaveBeenCalledOnce();
  expect(posts).toHaveBeenCalledOnce();
});

it("keeps reading text while a context-reset history request is pending", async () => {
  const page = history();
  page.messages = page.messages.slice(0, 1);
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  let resolveHistory!: (response: Response) => void;
  let deferred = false;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? new Response(new ReadableStream({ start(value) { controller = value; } }))
    : deferred ? new Promise<Response>((resolve) => { resolveHistory = resolve; }) : json(page);
  const transport = new NyxAgentTransport();
  await transport.history(id);
  deferred = true;
  const sending = transport.send(id, "hello", vi.fn());
  await vi.waitFor(() => expect(controller).toBeDefined());
  controller.enqueue(sseBytes(runningStatus,
    { event: "turn.notice", code: "context_reset", message: "Reset" },
    { event: "message.started", message_id: "reply", role: "assistant" },
    { event: "block.started", message_id: "reply", block_id: "text", index: 0, block: { type: "text", block_id: "text", text: "" } },
    { event: "block.delta", block_id: "text", text: "Text is flowing" },
  ));
  await vi.waitFor(() => expect(transport.session(id).messages.at(-1)?.content).toBe("Text is flowing"));
  expect(resolveHistory).toBeDefined();
  deferred = false;
  resolveHistory(json(page));
  controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify({ event: "turn.completed", cursor: 6, turn_id: "turn", status: "completed", error: null })}\n\n`));
  controller.close();
  await sending;
});

it("does not restore an active turn from a history request older than settlement", async () => {
  const active = history();
  active.messages = active.messages.slice(0, 1);
  active.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [], attachments: [] };
  let resolveOld!: (response: Response) => void;
  let calls = 0;
  globalThis.__nyxidAssistantHttpMock = () => ++calls === 1
    ? new Promise<Response>((resolve) => { resolveOld = resolve; }) : json(history());
  const transport = new NyxAgentTransport();
  const old = transport.history(id);
  await vi.waitFor(() => expect(resolveOld).toBeDefined());
  await transport.history(id);
  resolveOld(json(active));
  await old;
  expect(transport.getActiveTurnId(id)).toBeUndefined();
  expect(transport.session(id).messages.at(-1)?.content).toBe("Answer 界");
});

it.each([null, undefined])("parses backend failures with status %s through active polling, settlement and reload", async (status) => {
  let active = true;
  const attachment = { id: "upload", origin: "user_upload", content_type: "text/plain", size: 5, label: "source.txt", pages: null, image_input: null, expired: false };
  const activity = { id: "failed", label: "tool", status: "error", started_at: start, ended_at: reset, machine: null, failure: { kind: "tool_error", ...(status === null ? { status: null } : {}) } };
  globalThis.__nyxidAssistantHttpMock = () => {
    const page = history();
    const wire = {
      ...page,
      conversation: { ...page.conversation, active_turn: active ? { turn_id: "turn", started_at: start, activities: [activity], attachments: [attachment], activity_total: null } : null },
      messages: active ? page.messages.slice(0, 1) : [{ ...page.messages[0] }, { ...page.messages[1], activities: [activity], attachments: [attachment], activity_total: null }],
    };
    const parsed = nyxAgentHistorySchema.parse(wire);
    const failure = active ? parsed.conversation.active_turn?.activities[0]?.failure : parsed.messages[1]?.activities[0]?.failure;
    expect(failure?.status).toBeUndefined();
    return json(wire);
  };
  const transport = new NyxAgentTransport();
  await transport.history(id);
  await transport.history(id);
  expect(transport.isRunning(id)).toBe(true);
  expect(transport.session(id).messages.at(-1)?.toolCalls?.[0]?.failure).toEqual({ kind: "tool_error", ...(status === null ? { status: undefined } : {}) });
  active = false;
  await transport.history(id);
  expect(transport.isRunning(id)).toBe(false);
  expect(transport.session(id).status).toBe("completed_text");
  expect(transport.session(id).messages.at(-1)?.toolCalls?.[0]?.status).toBe("error");
  const reloaded = new NyxAgentTransport();
  await reloaded.history(id);
  expect(reloaded.isRunning(id)).toBe(false);
  expect(reloaded.session(id).messages.at(-1)?.content).toBe("Answer 界");
  expect(reloaded.session(id).messages.at(-1)?.toolCalls?.[0]?.failure?.kind).toBe("tool_error");
});

it.each(["", "Partial reply"])("keeps stopped replies visible live and after transcript reload: %s", async (text) => {
  let stopped = false;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    if (endpoint.endsWith("/stop")) { stopped = true; return new Response(null, { status: 204 }); }
    const page = history();
    if (!stopped) {
      page.messages.pop();
      page.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [], attachments: [] };
    } else {
      page.messages[1] = { ...page.messages[1]!, text, status: "failed", error_code: "cancelled" };
    }
    return json(page);
  };
  const assertVisible = (transport: NyxAgentTransport) => {
    const session = transport.session(id);
    expect(session.status).toBe("stopped");
    expect(session.messages.at(-1)?.status).toBe("stopped");
    const view = render(createElement(ChatMessageList, { session, bottomInset: 0 }));
    expect(screen.getByRole("status", { name: "Turn stopped" })).toHaveTextContent("Stopped");
    if (text) expect(screen.getByText(text)).toBeVisible();
    expect(view.container.querySelector("[data-empty-turn-error]")).toBeNull();
    view.unmount();
  };
  const transport = new NyxAgentTransport();
  await transport.history(id);
  await transport.stop(id);
  await transport.history(id);
  assertVisible(transport);
  const reloaded = new NyxAgentTransport();
  await reloaded.history(id);
  assertVisible(reloaded);
});

function sseBytes(...events: Record<string, unknown>[]): Uint8Array {
  return new TextEncoder().encode(
    events.map((event, i) => `data: ${JSON.stringify({ ...event, cursor: i + 1 })}\n\n`).join(""),
  );
}

it.each(["", "Partial stopped reply"])("retains stopped row identity during the next live turn and settlement: %s", async (text) => {
  const page = history();
  page.messages[1] = {
    ...page.messages[1]!,
    text,
    status: "failed",
    error_code: "cancelled",
  };
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  let cursor = 0;
  const emit = (...events: Record<string, unknown>[]) => {
    controller.enqueue(new TextEncoder().encode(events.map((event) =>
      `data: ${JSON.stringify({ ...event, cursor: ++cursor })}\n\n`,
    ).join("")));
  };
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    if (endpoint.endsWith("/turns")) {
      page.conversation.active_turn = { turn_id: "next-turn", started_at: afterReset, activities: [], attachments: [] };
      page.messages.push({ ...page.messages[0]!, id: "next-user", seq: 3, turn_id: "next-turn", text: "Continue" });
      return new Response(new ReadableStream({
        start(value) {
          controller = value;
          emit(
            { ...runningStatus, turn_id: "next-turn" },
            { event: "message.started", message_id: "next-assistant", role: "assistant" },
            { event: "block.started", message_id: "next-assistant", index: 0, block_id: "next-block", block: { type: "text", block_id: "next-block", text: "" } },
            { event: "block.delta", block_id: "next-block", text: "New reply" },
          );
        },
      }), { headers: { "content-type": "text/event-stream" } });
    }
    return endpoint.includes(`/conversations/${id}`)
      ? json(page)
      : json({ conversations: [page.conversation], next_cursor: null });
  };
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const stopped = transport.session(id).messages.find((message) => message.id === "a");
  expect(stopped).toMatchObject({ status: "stopped", errorCode: "cancelled", content: text });
  const sending = transport.send(id, "Continue", vi.fn());
  await vi.waitFor(() => expect(transport.session(id).messages.at(-1)).toMatchObject({ id: "next-assistant", status: "streaming", content: "New reply" }));
  const assertStopped = () => {
    const session = transport.session(id);
    expect(session.messages.find((message) => message.id === "a")).toBe(stopped);
    expect(stopped?.error).toBeUndefined();
    const view = render(createElement(ChatMessageList, { session, bottomInset: 0 }));
    expect(screen.getByRole("status", { name: "Turn stopped" })).toHaveTextContent("Stopped");
    view.unmount();
  };
  assertStopped();
  await transport.history(id);
  assertStopped();
  page.conversation.active_turn = null;
  page.messages.push({ ...page.messages[1]!, id: "next-assistant", seq: 4, turn_id: "next-turn", text: "New reply", status: "completed", error_code: null });
  emit(
    { event: "block.completed", block_id: "next-block", block: { type: "text", block_id: "next-block", text: "New reply" } },
    { event: "message.completed", message_id: "next-assistant" },
    { event: "turn.completed", turn_id: "next-turn", status: "completed", error: null },
  );
  controller.close();
  await sending;
  assertStopped();
  expect(transport.session(id).messages.at(-1)?.status).toBe("complete");
});

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
      const page = history();
      page.messages = page.messages.slice(0, 1);
      page.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [], attachments: [] };
      return endpoint.includes(`/conversations/${id}`)
        ? json(page)
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
      "The live reply was interrupted. NyxBot may still be working; check the chat before sending again.",
    ],
    ["an SSE payload that is not JSON", () => chunked(new TextEncoder().encode("data: broken\n\n")), "name", "SyntaxError"],
    ["a body that is not UTF-8", () => chunked(new Uint8Array([0xff, 0xfe, 0x0a])), "name", "TypeError"],
    [
      "a turn for another conversation",
      () => chunked(sseBytes({ ...runningStatus, conversation_id: other })),
      "message",
      "The live reply was interrupted. NyxBot may still be working; check the chat before sending again.",
    ],
    [
      "a stream that ends without a terminal event",
      () => chunked(sseBytes(runningStatus)),
      "message",
      "The live reply was interrupted. NyxBot may still be working; check the chat before sending again.",
    ],
    ["a 2xx without a body", () => new Response(null), "message", "The live reply was interrupted. NyxBot may still be working; check the chat before sending again."],
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
      expect((error as Error).message).toBe("The live reply was interrupted. NyxBot may still be working; check the chat before sending again.");
      expect(isAdmittedSendError(error)).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps the server trace when the accepted stream breaks", async () => {
    const error = await sendFailure(() => new Response(new ReadableStream({
      start(controller) { controller.error(new TypeError("network error")); },
    }), { headers: { "X-NyxID-Trace-Id": "9ad6d0fa-bb8f-489e-8fdd-f8e34960921b" } }));
    expect((error as Error).message).toBe(
      "The live reply was interrupted. NyxBot may still be working; check the chat before sending again.\nTrace ID: 9ad6d0fa-bb8f-489e-8fdd-f8e34960921b",
    );
    expect(isAdmittedSendError(error)).toBe(true);
  });

  it("preserves admission when a browser exception has a read-only message", async () => {
    const browserError = new Error("network error");
    Object.defineProperty(browserError, "message", { writable: false });
    const error = await sendFailure(() => new Response(new ReadableStream({
      start(controller) { controller.error(browserError); },
    })));
    expect((error as Error).message).toContain("NyxBot may still be working");
    expect(isAdmittedSendError(error)).toBe(true);
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
    expect((error as Error).message).toContain("your message may have arrived");
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

it("sends the chosen draft model only for a new thread and never changes the next draft's default", async () => {
  const transport = new NyxAgentTransport();
  const bodies: unknown[] = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (endpoint.endsWith("/turns")) {
      bodies.push(JSON.parse(String(init.body)));
      return stream();
    }
    if (endpoint.includes("/conversations?")) return json({ conversations: [conversation()], next_cursor: null });
    return json(history());
  };
  await transport.send(undefined, "Selected model", vi.fn(), undefined, { model: "nyxagent/research" });
  expect(bodies[0]).toEqual({ model: "nyxagent/research", text: "Selected model" });
  expect(transport.getModel()).toBe("nyxagent/chat");
  await transport.send(id, "Continue", vi.fn(), undefined, { model: "nyxagent/research" });
  expect(bodies[1]).toEqual({ conversation_id: id, text: "Continue" });
});

it("inherits a specialist's configured profile unless the owner explicitly selects a model", async () => {
  const transport = new NyxAgentTransport();
  const bodies: unknown[] = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (endpoint.endsWith("/turns")) {
      bodies.push(JSON.parse(String(init.body)));
      return stream();
    }
    if (endpoint.includes("/conversations?")) return json({ conversations: [conversation()], next_cursor: null });
    return json(history());
  };
  const agent = { id: "specialist-id", kind: "specialist" as const, name: "faq", destroyed: false };
  await transport.send(undefined, "Use your default", vi.fn(), undefined, { agent });
  expect(bodies[0]).toEqual({ agent_id: agent.id, text: "Use your default" });
  await transport.send(undefined, "Use this model", vi.fn(), undefined, { agent, model: "nyxagent/research" });
  expect(bodies[1]).toEqual({ agent_id: agent.id, model: "nyxagent/research", text: "Use this model" });
});

it("persists decision continuation provenance through the request, optimistic turn and history without losing its turn count", async () => {
  const transport = new NyxAgentTransport();
  const card = "11111111-1111-4111-8111-111111111111";
  const bodies: unknown[] = [];
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const stored = history();
  stored.messages[0] = { ...stored.messages[0]!, continuation_of: card };
  globalThis.__nyxidAssistantHttpMock = async ({ endpoint, init }) => {
    if (endpoint.endsWith("/turns")) {
      bodies.push(JSON.parse(String(init.body))); await gate; return stream();
    }
    if (endpoint.includes("/conversations?")) return json({ conversations: [conversation()], next_cursor: null });
    return json(stored);
  };
  const sending = transport.send(id, "Continue after the decision", vi.fn(), undefined, { acknowledgementId: card });
  await vi.waitFor(() => expect(bodies).toHaveLength(1));
  expect(bodies[0]).toMatchObject({ acknowledgement_id: card });
  expect(transport.session(id).messages.find((row) => row.role === "user")?.continuation_of).toBe(card);
  expect(transport.session(id).expectedTurnCount).toBe(1);
  release(); await sending;
  expect(transport.session(id).messages.find((row) => row.id === "u")?.continuation_of).toBe(card);
  expect(transport.session(id).expectedTurnCount).toBe(1);
});


it.each([
  ["wallet_suspended", "Contact support"],
  ["plan_entitlement_required", "Check your plan"],
  ["model_interrupted", "Some actions may already have completed"],
  ["model_unavailable", "Choose another model"],
])("provides specific remedy copy for %s", (code, copy) => expect(storedError(code)).toContain(copy));

it("carries upstream code from history to the diagnostic view", async () => {
  const page = history();
  page.messages[1] = { ...page.messages[1]!, status: "failed", error_code: "model_interrupted",
    upstream_error_code: "turn_failed", error_detail: { stage: "stream", http_status: null, upstream_request_id: null, response_id: null, continuations: 0, turn_id: "turn", occurred_at: start } };
  globalThis.__nyxidAssistantHttpMock = () => json(page);
  const transport = new NyxAgentTransport();
  await transport.history(id);
  expect(transport.session(id).messages[1]?.errorDetail?.upstreamCode).toBe("turn_failed");
});

it.each([
  ["insufficient_credits", "Add credits in Billing & Usage"],
  ["wallet_suspended", "Your billing wallet is suspended"],
  ["plan_entitlement_required", "Your plan does not include this service"],
  ["billing_not_configured", "Billing setup is incomplete"],
  ["billing_provider_unavailable", "billing service is unavailable"],
])("keeps the billing cause %s visible in the live terminal and after reload", async (code, copy) => {
  const page = history();
  const transport = new NyxAgentTransport();
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? chunked(sseBytes(
        runningStatus,
        { event: "message.started", message_id: "live-billing-reply", role: "assistant" },
        { event: "turn.completed", turn_id: "turn", status: "failed", error: {
          code, message: "Generic upstream failure", upstream_error_code: "nyxid_error", stage: "stream", http_status: 502,
        } },
      ))
    : json(page);
  await transport.history(id);
  const views: import("@/lib/assistant/chat-types").ChatMessage[] = [];
  const unsubscribe = transport.subscribe(() => {
    const tail = transport.session(id).messages.at(-1);
    if (tail) views.push(tail);
  });
  await transport.send(id, "hello", vi.fn());
  unsubscribe();
  expect(views.some((view) => view.status === "error" && view.errorCode === code && view.error?.includes(copy))).toBe(true);
  page.messages[1] = { ...page.messages[1]!, text: "", status: "failed", error_code: code,
    upstream_error_code: "nyxid_error", error_detail: {
      stage: "stream", http_status: 502, upstream_request_id: null, response_id: null, continuations: 0, turn_id: "turn", occurred_at: start,
    } };
  const reloaded = new NyxAgentTransport();
  await reloaded.history(id);
  const message = reloaded.session(id).messages[1];
  expect(message).toMatchObject({ status: "error", errorCode: code, errorDetail: { upstreamCode: "nyxid_error" } });
  expect(message?.error).toContain(copy);
  render(createElement(ChatMessageList, { session: reloaded.session(id), bottomInset: 0 }));
  expect(screen.getByRole("alert")).toHaveTextContent(copy);
});

it("exposes live upstream diagnostics and capacity status before history refresh", async () => {
  const page = history();
  page.messages[1] = { ...page.messages[1]!, turn_metadata: { attempts: 1, capacity_wait_ms: 0 } };
  const transport = new NyxAgentTransport();
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? chunked(sseBytes(
        runningStatus,
        { event: "message.started", message_id: "live-reply", role: "assistant" },
        { ...runningStatus, status: "waiting", waiting_for_capacity: true, capacity_wait_note_origin: "nyxid_system", engine_request_started_at: start },
        { ...runningStatus, status: "running", waiting_for_capacity: false, engine_accepted: true },
        { event: "turn.completed", turn_id: "turn", status: "failed", error: {
          attempts: 7, capacity_wait_ms: 80_000, code: "model_interrupted", message: "fixed server copy", upstream_error_code: "turn_failed", stage: "stream", http_status: 200,
        } },
      ))
    : json(page);
  await transport.history(id);
  const views: import("@/lib/assistant/chat-types").ChatMessage[] = [];
  const unsubscribe = transport.subscribe(() => {
    const messages = transport.session(id).messages;
    const previous = messages.find((message) => message.id === page.messages[1]!.id);
    if (previous) expect(previous.turnMetadata?.attempts).toBe(1);
    const tail = messages.at(-1);
    if (tail) views.push(tail);
  });
  await transport.send(id, "hello", vi.fn());
  unsubscribe();
  expect(views.some((view) => view.waitingForCapacity && view.systemNote === "capacity_wait")).toBe(true);
  expect(views.some((view) => view.status === "streaming" && !view.systemNote)).toBe(true);
  expect(views.some((view) => view.errorDetail?.upstreamCode === "turn_failed" && view.errorDetail.attempts === 7 && view.errorDetail.capacityWaitMs === 80_000 && view.status === "error")).toBe(true);
});

it("restores capacity timestamps and attempt diagnostics from persisted history", async () => {
  const page = history();
  page.conversation.active_turn = { turn_id: "waiting", started_at: start, activities: [], attachments: [], waiting_for_capacity: true,
    capacity_wait_note_origin: "nyxid_system", engine_request_started_at: start, capacity_wait_started_at: start, capacity_wait_deadline: "2026-09-17T00:02:00Z" };
  page.messages = [page.messages[0]!];
  globalThis.__nyxidAssistantHttpMock = () => json(page);
  const transport = new NyxAgentTransport();
  await transport.history(id);
  expect(transport.session(id).messages.at(-1)).toMatchObject({ waitingForCapacity: true, systemNote: "capacity_wait", content: "",
    capacityWaitStartedAt: start, capacityWaitDeadline: "2026-09-17T00:02:00Z" });
  page.conversation.active_turn = null;
  page.messages.push({ ...history().messages[1]!, turn_metadata: { attempts: 7, capacity_wait_ms: 80_000 } });
  await transport.history(id);
  expect(transport.session(id).messages.at(-1)?.turnMetadata).toEqual({ attempts: 7, capacityWaitMs: 80_000 });
  expect(storedError("capacity_exceeded", 120_000)).toContain("waited 2 minutes");
  expect(storedError("capacity_exceeded", 119_999)).toContain("waited 2 minutes");
  expect(storedError("capacity_exceeded", 120_001)).toContain("waited 2 minutes 1 second");
});

it("shows only the server-authorized pending note during an unanswered admission and removes it after Stop", async () => {
  const page = history();
  page.messages = [page.messages[0]!];
  page.conversation.active_turn = { turn_id: "slow", started_at: start, activities: [], attachments: [],
    waiting_for_capacity: false, engine_request_started_at: start, capacity_wait_deadline: "2026-09-17T00:01:49Z" };
  globalThis.__nyxidAssistantHttpMock = () => json(page);
  const transport = new NyxAgentTransport();
  await transport.history(id);
  expect(transport.session(id).messages.at(-1)?.systemNote).toBeUndefined();
  page.conversation.active_turn.capacity_wait_note_origin = "nyxid_system";
  await transport.history(id);
  expect(transport.session(id).messages.at(-1)).toMatchObject({ systemNote: "capacity_wait", content: "", capacityWaitStartedAt: start, capacityWaitDeadline: "2026-09-17T00:01:49Z" });
  await transport.stop(id);
  page.conversation.active_turn = null;
  page.messages.push({ ...history().messages[1]!, text: "", status: "failed", error_code: "cancelled" });
  await transport.history(id);
  expect(transport.session(id).messages.every((message) => !message.systemNote)).toBe(true);
});

it("never restores the pending acknowledgement from a poll that arrives after live acceptance", async () => {
  const stale = history();
  stale.messages = [stale.messages[0]!];
  stale.conversation.active_turn = { turn_id: "turn", started_at: start, activities: [], attachments: [], waiting_for_capacity: true,
    engine_request_started_at: start, capacity_wait_note_origin: "nyxid_system", capacity_wait_deadline: "2026-09-17T00:02:00Z" };
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  let resolvePoll!: (response: Response) => void;
  let deferPoll = false;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? new Response(new ReadableStream({ start(value) { controller = value; } }), { headers: { "content-type": "text/event-stream" } })
    : deferPoll ? new Promise<Response>((resolve) => { resolvePoll = resolve; }) : json(history());
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const sending = transport.send(id, "hello", vi.fn());
  await vi.waitFor(() => expect(controller).toBeDefined());
  const emit = (event: Record<string, unknown>, cursor: number) => controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify({ ...event, cursor })}\n\n`));
  emit(runningStatus, 1);
  emit({ event: "message.started", message_id: "live-reply", role: "assistant" }, 2);
  emit({ ...runningStatus, status: "waiting", waiting_for_capacity: true, capacity_wait_note_origin: "nyxid_system", engine_request_started_at: start }, 3);
  await vi.waitFor(() => expect(transport.session(id).messages.at(-1)?.systemNote).toBe("capacity_wait"));
  deferPoll = true;
  const polling = transport.history(id);
  await vi.waitFor(() => expect(resolvePoll).toBeDefined());
  emit({ ...runningStatus, status: "running", waiting_for_capacity: false, engine_accepted: true }, 4);
  await vi.waitFor(() => expect(transport.session(id).messages.at(-1)?.systemNote).toBeUndefined());
  resolvePoll(json(stale));
  await polling;
  expect(transport.session(id).messages.at(-1)?.systemNote).toBeUndefined();
  expect(transport.getHistory(id)?.conversation.active_turn?.waiting_for_capacity).toBe(false);
  deferPoll = false;
  emit({ event: "turn.completed", turn_id: "turn", status: "completed", error: null }, 5);
  controller.close();
  await sending;
});


it("shows a queued turn as queued until it runs", async () => {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? new Response(new ReadableStream({ start(value) { controller = value; } }), { headers: { "content-type": "text/event-stream" } })
    : json(history());
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const sending = transport.send(id, "hello", vi.fn());
  await vi.waitFor(() => expect(controller).toBeDefined());
  const emit = (event: Record<string, unknown>, cursor: number) => controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify({ ...event, cursor })}\n\n`));
  emit({ ...runningStatus, status: "waiting", queued: true, waiting_for_capacity: true,
    capacity_wait_started_at: start, capacity_wait_deadline: "2026-09-17T00:10:00Z" }, 1);
  emit({ event: "message.started", message_id: "live-reply", role: "assistant" }, 2);
  await vi.waitFor(() => expect(transport.session(id).messages.at(-1)).toMatchObject({
    queued: true, waitingForCapacity: true, capacityWaitStartedAt: start, capacityWaitDeadline: "2026-09-17T00:10:00Z",
  }));
  emit({ ...runningStatus, status: "running", waiting_for_capacity: false }, 3);
  await vi.waitFor(() => expect(transport.getConversation(id)?.active_turn?.queued).toBe(false));
  expect(transport.session(id).messages.at(-1)?.queued).toBeFalsy();
  expect(transport.session(id).messages.at(-1)?.waitingForCapacity).toBeFalsy();
  emit({ event: "turn.completed", turn_id: "turn", status: "completed", error: null }, 4);
  controller.close();
  await sending;
});

it("describes a busy session separately from platform capacity", () => {
  expect(storedError("session_busy", 120_000)).toBe("The assistant session is busy. Try again shortly.");
  expect(storedError("capacity_exceeded", 120_000)).toContain("at capacity right now (waited 2 minutes)");
});

it("starts new turn status without the previous turn's activity or wait clock", async () => {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  let page = history();
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? new Response(new ReadableStream({ start(value) { controller = value; } }), { headers: { "content-type": "text/event-stream" } })
    : json(page);
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const sending = transport.send(id, "hello", vi.fn());
  await vi.waitFor(() => expect(controller).toBeDefined());
  const emit = (event: Record<string, unknown>, cursor: number) => controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify({ ...event, cursor })}\n\n`));
  emit(runningStatus, 1);
  emit({ event: "message.started", message_id: "live-reply", role: "assistant" }, 2);
  await vi.waitFor(() => expect(transport.getConversation(id)?.active_turn?.turn_id).toBe("turn"));
  const prior = {
    turn_id: "turn", started_at: start,
    activities: [{ id: "old-tool", label: "github__read", status: "completed" as const, started_at: start, ended_at: null }],
    attachments: [{ id: "old-image", content_type: "image/png" as const, size: 10, label: "old" }],
    waiting_for_capacity: true, capacity_wait_note_origin: "nyxid_system", engine_request_started_at: start,
    capacity_wait_started_at: start, capacity_wait_deadline: afterReset,
  };
  page = { ...page, conversation: { ...page.conversation, active_turn: prior } };
  await transport.history(id);
  emit({ ...runningStatus, status: "waiting", waiting_for_capacity: true }, 3);
  await vi.waitFor(() => expect(transport.getConversation(id)?.active_turn?.capacity_wait_note_origin).toBe("nyxid_system"));
  expect(transport.getConversation(id)?.active_turn).toMatchObject(prior);
  emit({ ...runningStatus, turn_id: "next-turn", status: "waiting", waiting_for_capacity: true }, 4);
  await vi.waitFor(() => expect(transport.getConversation(id)?.active_turn?.turn_id).toBe("next-turn"));
  const next = transport.getConversation(id)?.active_turn;
  expect(next?.activities).toEqual([]);
  expect(next?.attachments).toEqual([]);
  expect(next?.started_at).not.toBe(start);
  expect(next?.engine_request_started_at).toBeUndefined();
  expect(next?.capacity_wait_started_at).toBeUndefined();
  expect(next?.capacity_wait_deadline).toBeUndefined();
  expect(next?.capacity_wait_note_origin).toBeUndefined();
  expect(transport.session(id).messages.at(-1)?.toolCalls).toBeUndefined();
  emit({ event: "turn.completed", turn_id: "next-turn", status: "completed", error: null }, 5);
  page = history();
  controller.close();
  await sending;
});

function creationReceipt(turn: string | null, status: "pending" | "allowed" | "denied" = "allowed") {
  return {
    id: "12345678-1234-4123-8123-123456789012", kind: "action" as const, status,
    summary: "Review the draft", decider: "user" as const, decided_by: "user" as const,
    reason: null, service_slug: null, service_name: null, tool_name: "draft_agent_skill",
    created_at: reset, decided_at: afterReset, expires_at: afterReset, creation_turn_id: turn,
  };
}
it("folds settled decisions into the exact assistant event turn even when the initiating user is older", async () => {
  const transport = new NyxAgentTransport();
  const page = history();
  page.messages.push({ ...page.messages[1]!, id: "event-answer", turn_id: "event-turn", seq: 3, created_at: "2026-09-17T00:00:05Z" });
  const receipt = creationReceipt("event-turn");
  page.acknowledgements = [receipt];
  globalThis.__nyxidAssistantHttpMock = () => json(page);
  await transport.history(id);
  // No standalone row: the decision rides on its reply's turn summary.
  expect(transport.session(id).messages.map((m) => m.id)).toEqual(["u", "a", "event-answer"]);
  const first = transport.session(id).messages.at(-1)!.decisions;
  expect(first?.map((row) => row.id)).toEqual([receipt.id]);
  // An unchanged refresh keeps the folded rows by reference.
  await transport.history(id);
  expect(transport.session(id).messages.at(-1)!.decisions).toBe(first);
  // A decision-state change is reflected on the same reply.
  page.acknowledgements = [{ ...receipt, status: "denied" }];
  await transport.history(id);
  expect(transport.session(id).messages.map((m) => m.id)).toEqual(["u", "a", "event-answer"]);
  expect(transport.session(id).messages.at(-1)!.decisions?.[0]?.status).toBe("denied");
});
it("keeps a pending card as its own row beside the reply it was raised in", async () => {
  const transport = new NyxAgentTransport();
  const page = history();
  page.messages.push({ ...page.messages[1]!, id: "event-answer", turn_id: "event-turn", seq: 3, created_at: "2026-09-17T00:00:05Z" });
  const card = { ...creationReceipt("event-turn"), status: "pending" as const, decided_by: null, decided_at: null };
  page.acknowledgements = [card];
  globalThis.__nyxidAssistantHttpMock = () => json(page);
  await transport.history(id);
  expect(transport.session(id).messages.map((m) => m.id)).toContain(`nyxagent-acknowledgement:${card.id}`);
  expect(transport.session(id).messages.find((m) => m.id === "event-answer")?.decisions).toBeUndefined();
});
it("folds an automatic receipt into the live assistant placeholder then the settled answer", async () => {
  const transport = new NyxAgentTransport();
  const page = history();
  page.conversation.active_turn = { turn_id: "event-turn", started_at: afterReset, activities: [], attachments: [] };
  const receipt = creationReceipt("event-turn");
  page.acknowledgements = [receipt];
  globalThis.__nyxidAssistantHttpMock = () => json(page);
  await transport.history(id);
  let messages = transport.session(id).messages;
  expect(messages.map((m) => m.id)).toEqual(["u", "a", "event-turn-assistant"]);
  expect(messages.at(-1)!.decisions?.map((row) => row.id)).toEqual([receipt.id]);
  page.conversation.active_turn = null;
  page.messages.push({ ...page.messages[1]!, id: "event-answer", turn_id: "event-turn", seq: 3 });
  await transport.history(id);
  messages = transport.session(id).messages;
  expect(messages.map((m) => m.id)).toEqual(["u", "a", "event-answer"]);
  expect(messages.at(-1)!.decisions?.map((row) => row.id)).toEqual([receipt.id]);
});
it.each([null, "unloaded-turn"])("keeps a linked receipt with %s anchor standalone and deduplicated", async (turn) => {
  const transport = new NyxAgentTransport();
  const page = history();
  const receipt = creationReceipt(turn);
  page.acknowledgements = [receipt];
  page.messages = [page.messages[1]!];
  globalThis.__nyxidAssistantHttpMock = () => json(page);
  await transport.history(id, undefined, receipt.id);
  await transport.history(id, undefined, receipt.id);
  expect(transport.session(id).messages.map((m) => m.id)).toEqual([`nyxagent-acknowledgement:${receipt.id}`, "a"]);
});


describe("Nyxbot native error telemetry", () => {
  it.each([[403, "NYXBOT_HTTP_FORBIDDEN"], [500, "NYXBOT_HTTP_SERVER_ERROR"]])(
    "reports HTTP %s once with the server trace and no private text", async (status, code) => {
      const trace = "76dd13d5-e7cf-4c63-a33b-26036d2a495d";
      globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
        ? new Response(JSON.stringify({ error_code: 1000, message: "private server diagnostic" }), {
            status, headers: { "X-NyxID-Trace-Id": trace },
          })
        : json(history());
      await expect(new NyxAgentTransport().send(id, "private prompt", vi.fn())).rejects.toThrow();
      expect(capture).toHaveBeenCalledExactlyOnceWith({ name: "nyxbot.error", props: expect.objectContaining({
        error_code: code, engine: "nyxagent", stage: "request", http_status: status,
        trace_id: trace, conversation_id: id, upstream_error_code: 1000,
      }) });
      expect(JSON.stringify(vi.mocked(capture).mock.calls)).not.toContain("private");
    },
  );

  it.each(["model_interrupted", "cancelled", null])("records live failure %s without recounting it on reload", async (code) => {
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
      ? chunked(sseBytes(runningStatus, { event: "turn.completed", turn_id: "turn", status: "failed",
          error: code ? { code, message: "private runtime detail" } : null }))
      : json(history());
    const transport = new NyxAgentTransport();
    await transport.send(id, "private prompt", vi.fn());
    await transport.history(id);
    expect(capture).toHaveBeenCalledTimes(code === "cancelled" ? 0 : 1);
    if (code !== "cancelled") expect(capture).toHaveBeenCalledWith({ name: "nyxbot.error", props: expect.objectContaining({
      error_code: "NYXBOT_RUN_ERROR", engine: "nyxagent", stage: "stream", http_status: 200,
      upstream_error_code: code ?? "other", turn_id: "turn",
    }) });
    expect(JSON.stringify(vi.mocked(capture).mock.calls)).not.toContain("private");
  });

  it("reports an admission deadline once and never replays the submitted turn", async () => {
    vi.useFakeTimers();
    let turns = 0;
    globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
      if (endpoint.endsWith("/turns")) { turns++; return new Promise<Response>(() => undefined); }
      return json(history());
    };
    try {
      const pending = new NyxAgentTransport().send(id, "Question", vi.fn());
      const rejected = expect(pending).rejects.toThrow();
      await vi.advanceTimersByTimeAsync(45_001);
      await rejected;
      expect(turns).toBe(1);
      expect(capture).toHaveBeenCalledExactlyOnceWith({ name: "nyxbot.error", props: expect.objectContaining({
        error_code: "NYXBOT_START_TIMEOUT", engine: "nyxagent", stage: "request",
      }) });
    } finally { vi.useRealTimers(); }
  });
});

// Value: protects=live failed history outcomes emit once even when the reader is aborted;
// fails_when=history settlement is mistaken for user cancellation; why_new=SSE terminal fixtures do not cover history;
// seam=none; no_claim=live network or PostHog delivery.
it.each(["model_interrupted", null, "cancelled"])("reports history-recovered live failure %s without recounting reload", async (code) => {
  const page = history();
  page.messages = page.messages.slice(0, 1);
  let reader!: ReadableStreamDefaultController<Uint8Array>;
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? new Response(new ReadableStream({ start(value) { reader = value; } })) : json(page);
  const transport = new NyxAgentTransport();
  await transport.history(id);
  const sending = transport.send(id, "Question", vi.fn());
  await vi.waitFor(() => expect(reader).toBeDefined());
  reader.enqueue(sseBytes(runningStatus, { event: "message.started", message_id: "recovered", role: "assistant" }));
  await vi.waitFor(() => expect(transport.getActiveTurnId(id)).toBe("turn"));
  page.messages.push({ ...history().messages[1]!, id: "recovered", status: "failed", error_code: code });
  await transport.history(id);
  await sending;
  await transport.history(id);
  expect(capture).toHaveBeenCalledTimes(code === "cancelled" ? 0 : 1);
  if (code !== "cancelled") expect(capture).toHaveBeenCalledWith({ name: "nyxbot.error", props: expect.objectContaining({
    error_code: "NYXBOT_RUN_ERROR", upstream_error_code: code ?? "other", engine: "nyxagent", stage: "stream",
  }) });
});


it.each([
  { source: "nyxid", stage: "model_proxy", code: "api_key_scope_forbidden", http_status: 403, nyxid_error_code: 9000 },
  { source: "nyxagent", stage: "stream", code: "turn_failed", http_status: null, nyxid_error_code: null },
])("keeps sanitized failure metadata live and after reload ($source)", async (failure) => {
  const page = history();
  const transport = new NyxAgentTransport();
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => endpoint.endsWith("/turns")
    ? chunked(sseBytes(runningStatus,
        { event: "message.started", message_id: "failure-reply", role: "assistant" },
        { event: "turn.completed", turn_id: "turn", status: "failed",
          turn_metadata: { failure_reason: failure },
          error: { code: "assistant_unavailable", message: "fixed copy", upstream_error_code: "turn_failed", stage: "stream" } },
      ))
    : json(page);
  await transport.history(id);
  const views: import("@/lib/assistant/chat-types").ChatMessage[] = [];
  const unsubscribe = transport.subscribe(() => {
    const tail = transport.session(id).messages.at(-1);
    if (tail) views.push(tail);
  });
  await transport.send(id, "hello", vi.fn());
  unsubscribe();
  expect(views.some((view) => view.errorDetail?.failureReason?.code === failure.code)).toBe(true);
  page.messages[1] = { ...page.messages[1]!, status: "failed", error_code: "assistant_unavailable",
    turn_metadata: { failure_reason: failure }, upstream_error_code: "turn_failed",
    error_detail: { stage: "stream", http_status: 200, upstream_request_id: null, response_id: null,
      continuations: 0, turn_id: "turn", occurred_at: start } };
  const reloaded = new NyxAgentTransport();
  await reloaded.history(id);
  expect(reloaded.session(id).messages[1]?.errorDetail?.failureReason).toEqual(failure);
});
