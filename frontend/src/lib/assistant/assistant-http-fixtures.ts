import type { AssistantHttpMockHandler } from "@/lib/assistant/assistant-http";
import {
  activeTaskFixture,
  assistantFixtureFrames,
  createSeededAssistantFixtureConversations,
  FIXTURE_REPLY,
  type AssistantFixtureConversation,
} from "@/lib/assistant/assistant-http-fixture-data";
import { matchAssistantHttpScenario } from "@/lib/assistant/assistant-http-scenarios";
import { trimChatTitle } from "@/lib/assistant/chat-session-state";
import type { StoredChatMessage } from "@/lib/assistant/chat-types";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";

type JsonRecord = Record<string, unknown>;

export interface AssistantHttpFixtureFaults {
  readonly historyDelayMs?: number;
  readonly historyErrorStatus?: number;
  readonly sendSilent?: boolean;
  readonly aliasOnFirstSend?: boolean;
  readonly firstEventSilenceMs?: number;
  readonly progressStallMs?: number;
  readonly stateEnvelopeSequence?: readonly unknown[];
  readonly unauthorized?: "coded" | "uncoded";
}

interface AssistantHttpFixtureGlobals {
  __nyxidAssistantHttpFaults?: AssistantHttpFixtureFaults;
  __nyxidAssistantHttpFixtureWorld?: AssistantHttpFixtureWorld;
}

declare global {
  var __nyxidAssistantHttpFaults: AssistantHttpFixtureFaults | undefined;
  var __nyxidAssistantHttpFixtureWorld: AssistantHttpFixtureWorld | undefined;
}

const STREAM_CADENCE_MS = 70;
const EMPTY_TURN_SETTLE_MS = 900;

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : {};
}

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function errorResponse(status: number, faults: AssistantHttpFixtureFaults): Response {
  const sessionCode = faults.unauthorized === "coded" ? 1001 : 0;
  return json(
    {
      error: status === 401 ? "unauthorized" : "mock_history_fault",
      error_code: status === 401 ? sessionCode : 0,
      message:
        status === 404
          ? "The requested conversation was not found."
          : status === 401
            ? "The assistant session is not authorized."
            : "Injected assistant HTTP fixture failure.",
    },
    status,
  );
}

function requestBody(init: RequestInit): JsonRecord {
  if (typeof init.body !== "string") return {};
  try {
    return asRecord(JSON.parse(init.body) as unknown);
  } catch {
    return {};
  }
}

function waitFor(ms: number, signal?: AbortSignal | null): Promise<void> {
  if (ms <= 0) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(resolve, ms);
    const abort = () => {
      clearTimeout(timer);
      reject(signal?.reason ?? new DOMException("Aborted", "AbortError"));
    };
    if (signal?.aborted) abort();
    else signal?.addEventListener("abort", abort, { once: true });
  });
}

function storedMessage(
  id: string,
  turnId: string,
  role: "user" | "assistant",
  content: string,
): StoredChatMessage {
  return {
    id,
    turnId,
    role,
    content,
    timestamp: Date.now(),
    status: "completed",
  };
}

function stateSnapshot(conversation: AssistantFixtureConversation): unknown {
  return {
    status: "current",
    stateVersion: conversation.stateVersion,
    snapshot: {
      actorId: conversation.meta.id,
      scopeId: `scope-${conversation.meta.id}`,
      stateVersion: conversation.stateVersion,
      progressSequence: conversation.progressSequence,
      activeTurn: conversation.activeTurn,
      latestTurn: conversation.latestTurn,
      recentTerminalTurns: [],
      activeTask: conversation.activeTask,
      pendingInput: conversation.pendingInput,
      pendingApproval: conversation.pendingApproval,
      controlFence: null,
      latestControlResult: null,
      latestStepControlResult: null,
      recentStepControlResults: [],
      continuationAdmission: null,
      latestInputResolution: null,
      latestApprovalResolution: conversation.latestApprovalResolution,
      pendingActions: conversation.pendingActions,
      recentActions: conversation.recentActions,
    },
  };
}

function sseLine(payload: string, index: number): string {
  const ending = index % 3 === 0 ? "\r\n\r\n" : index % 3 === 1 ? "\n\n" : "\r\r";
  return `data: ${payload}${ending}`;
}

function chunkedSse(
  frames: readonly unknown[],
  options: {
    readonly signal?: AbortSignal | null;
    readonly firstDelayMs?: number;
    readonly progressStallMs?: number;
    readonly onAbort?: () => void;
    readonly onComplete?: () => void;
  } = {},
): Response {
  const encoder = new TextEncoder();
  const timers = new Set<ReturnType<typeof setTimeout>>();
  let stopped = false;
  let streamController: ReadableStreamDefaultController<Uint8Array> | null = null;

  const stop = (aborted: boolean) => {
    if (stopped) return;
    stopped = true;
    for (const timer of timers) clearTimeout(timer);
    timers.clear();
    if (aborted) options.onAbort?.();
    else options.onComplete?.();
  };
  const abort = () => {
    stop(true);
    try {
      streamController?.error(
        options.signal?.reason ?? new DOMException("Aborted", "AbortError"),
      );
    } catch {
      // The reader may already have released the fixture stream.
    }
  };

  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      streamController = controller;
      const payloads = [
        ": fixture keepalive\n\n",
        ...frames.map((frame, index) =>
          sseLine(JSON.stringify(frame), index),
        ),
      ];
      payloads.splice(Math.min(4, payloads.length), 0, "data: {malformed\n\n");
      payloads.push("data: [DONE]\r\n\r\n");

      let elapsed = options.firstDelayMs ?? 40;
      payloads.forEach((payload, index) => {
        if (index === Math.floor(payloads.length / 2)) {
          elapsed += options.progressStallMs ?? 0;
        }
        const timer = setTimeout(() => {
          timers.delete(timer);
          if (stopped) return;
          const bytes = encoder.encode(payload);
          if (index % 4 === 2 && bytes.length > 4) {
            const split = Math.floor(bytes.length / 2);
            controller.enqueue(bytes.slice(0, split));
            controller.enqueue(bytes.slice(split));
          } else {
            controller.enqueue(bytes);
          }
          if (index === payloads.length - 1) {
            stop(false);
            controller.close();
          }
        }, elapsed);
        timers.add(timer);
        elapsed += STREAM_CADENCE_MS;
      });
    },
    cancel() {
      stop(true);
    },
  });
  if (options.signal?.aborted) abort();
  else options.signal?.addEventListener("abort", abort, { once: true });
  return new Response(body, {
    status: 200,
    headers: { "Content-Type": "text/event-stream" },
  });
}

export class AssistantHttpFixtureWorld {
  readonly conversations = new Map<string, AssistantFixtureConversation>();
  private nextConversation = 1;
  private nextTurn = 1;
  private stateEnvelopeOffset = 0;

  constructor() {
    for (const conversation of createSeededAssistantFixtureConversations()) {
      this.conversations.set(conversation.meta.id, conversation);
    }
  }

  readonly handler: AssistantHttpMockHandler = async ({ endpoint, init }) => {
    const faults = globalThis.__nyxidAssistantHttpFaults ?? {};
    const method = init.method ?? "GET";
    if (faults.unauthorized && faults.historyErrorStatus === 401) {
      return errorResponse(401, faults);
    }

    if (method === "GET" && endpoint === "/assistant/conversations") {
      return json({
        conversations: [...this.conversations.values()]
          .map((conversation) => conversation.meta)
          .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt)),
      });
    }

    const stateMatch = /^\/assistant\/conversations\/([^/?]+)\/state(?:\?|$)/.exec(endpoint);
    if (method === "GET" && stateMatch?.[1]) {
      const sequence = faults.stateEnvelopeSequence;
      if (sequence?.length) {
        const value = sequence[Math.min(this.stateEnvelopeOffset, sequence.length - 1)];
        this.stateEnvelopeOffset += 1;
        return json(value);
      }
      const conversation = this.conversations.get(decodeURIComponent(stateMatch[1]));
      return conversation ? json(stateSnapshot(conversation)) : json({ status: "not_found" }, 404);
    }

    const conversationMatch = /^\/assistant\/conversations\/([^/?]+)$/.exec(endpoint);
    if (conversationMatch?.[1]) {
      const conversationId = decodeURIComponent(conversationMatch[1]);
      if (method === "DELETE") {
        this.conversations.delete(conversationId);
        return new Response(null, { status: 204 });
      }
      if (method === "GET") {
        await waitFor(faults.historyDelayMs ?? 0, init.signal);
        if (faults.historyErrorStatus) {
          return errorResponse(faults.historyErrorStatus, faults);
        }
        const conversation = this.conversations.get(conversationId);
        return conversation
          ? json({
              messages: conversation.messages,
              stateVersion: conversation.stateVersion,
              projectionStatus: "current",
            })
          : errorResponse(404, faults);
      }
    }

    if (method === "POST" && endpoint === "/assistant/chat") {
      return this.handleChat(requestBody(init), init.signal, faults);
    }
    return undefined;
  };

  private handleChat(
    body: JsonRecord,
    signal: AbortSignal | null | undefined,
    faults: AssistantHttpFixtureFaults,
  ): Response {
    const type = typeof body.type === "string" ? body.type : "";
    if (type === "text" || type === "action.continue") {
      return this.streamCommand(body, signal, faults);
    }
    const conversationId = typeof body.conversationId === "string" ? body.conversationId : "";
    const conversation = this.conversations.get(conversationId);
    if (!conversation) return errorResponse(404, faults);
    conversation.stateVersion += 1;
    conversation.progressSequence += 1;
    if (type === "approval.resolve") {
      conversation.latestApprovalResolution = {
        approvalRequestId: body.requestId,
        outcome: body.approved === true ? "approved" : "rejected",
      };
      conversation.pendingApproval = null;
    } else if (type === "input.resolve") {
      conversation.pendingInput = null;
    } else if (type === "task.stop") {
      conversation.activeTurn = null;
      conversation.latestTurn = { turnId: body.turnId, status: "stopped" };
      conversation.activeTask = null;
    }
    return new Response(null, { status: 202 });
  }

  private streamCommand(
    body: JsonRecord,
    signal: AbortSignal | null | undefined,
    faults: AssistantHttpFixtureFaults,
  ): Response {
    const isText = body.type === "text";
    const requestedId = typeof body.conversationId === "string" ? body.conversationId : "";
    let conversation = requestedId ? this.conversations.get(requestedId) : undefined;
    if (!conversation) {
      const id = `nyxid-chat-mock-${String(this.nextConversation++)}`;
      const now = new Date().toISOString();
      conversation = {
        meta: {
          id,
          title: "New chat",
          createdAt: now,
          updatedAt: now,
          messageCount: 0,
          stateVersion: 1,
          taskStatus: null,
          attentionKind: null,
          attentionSince: null,
          activeStepSummary: null,
        },
        messages: [],
        pendingApproval: null,
        latestApprovalResolution: null,
        stateVersion: 1,
        progressSequence: 1,
        activeTurn: null,
        latestTurn: null,
        activeTask: null,
        pendingInput: null,
        pendingActions: [],
        recentActions: [],
      };
      this.conversations.set(id, conversation);
    }
    const actorId = conversation.meta.id;
    const turnId = `turn-mock-${String(this.nextTurn++)}`;
    const messageId = `assistant-message-${turnId}`;
    const prompt = isText && typeof body.prompt === "string" ? body.prompt : "NyxID action update";
    const scenarioState = useAssistantMockScenariosStore.getState();
    const scenario =
      isText && scenarioState.enabled
        ? matchAssistantHttpScenario(prompt, scenarioState.disabledScenarioIds)
        : null;
    if (isText) {
      conversation.messages.push(storedMessage(`user-message-${turnId}`, turnId, "user", prompt));
      if (conversation.meta.title === "New chat") {
        conversation.meta = { ...conversation.meta, title: trimChatTitle(prompt) };
      }
    }
    scenarioState.noteActivity({ scenarioId: scenario?.id ?? null, matched: Boolean(scenario), at: Date.now() });
    const output = scenario?.reply ?? FIXTURE_REPLY;
    conversation.stateVersion += 1;
    conversation.progressSequence += 1;
    conversation.activeTurn = { turnId, taskId: `task-${turnId}`, status: "active" };
    conversation.latestTurn = null;
    conversation.activeTask = activeTaskFixture(actorId, turnId);
    conversation.meta = {
      ...conversation.meta,
      messageCount: conversation.messages.length,
      stateVersion: conversation.stateVersion,
      taskStatus: "active",
      updatedAt: new Date().toISOString(),
    };

    const settle = (status: "completed" | "stopped") => {
      conversation.activeTurn = null;
      conversation.latestTurn = { turnId, status };
      conversation.activeTask = null;
      conversation.pendingInput = null;
      conversation.pendingApproval = null;
      conversation.stateVersion += 1;
      conversation.progressSequence = Math.max(conversation.progressSequence + 1, 101);
      if (status === "completed" && !faults.sendSilent) {
        conversation.messages.push(
          storedMessage(messageId, turnId, "assistant", output),
        );
      }
      conversation.meta = {
        ...conversation.meta,
        messageCount: conversation.messages.length,
        stateVersion: conversation.stateVersion,
        taskStatus: status,
        updatedAt: new Date().toISOString(),
      };
      if (status === "completed" && scenario?.serviceSlug) {
        useAssistantMockScenariosStore.getState().connectService(scenario.serviceSlug);
      }
    };

    const frames = faults.sendSilent
      ? [
          { runStarted: { actorId, runId: turnId, commandId: `command-${turnId}` } },
          { runFinished: { actorId, runId: turnId, result: {} } },
        ]
      : assistantFixtureFrames(actorId, turnId, messageId, output);
    return chunkedSse(frames, {
      signal,
      firstDelayMs:
        (faults.firstEventSilenceMs ?? 0) +
        (faults.sendSilent ? EMPTY_TURN_SETTLE_MS : 40),
      progressStallMs: faults.progressStallMs,
      onAbort: () => settle("stopped"),
      onComplete: () => settle("completed"),
    });
  }
}

export function installAssistantHttpFixtures(): AssistantHttpFixtureWorld {
  const globals = globalThis as typeof globalThis & AssistantHttpFixtureGlobals;
  const world = globals.__nyxidAssistantHttpFixtureWorld ?? new AssistantHttpFixtureWorld();
  globals.__nyxidAssistantHttpFixtureWorld = world;
  globalThis.__nyxidAssistantHttpMock = world.handler;
  useAssistantMockScenariosStore.getState().setEngineState("ready");
  return world;
}
