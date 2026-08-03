import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import aevatarNyxidChatStream from "@/lib/assistant/__fixtures__/aevatar-nyxid-chat-stream.sse?raw";
import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import {
  chatStreamClient,
  type ChatStreamRequestHandle,
} from "@/lib/assistant/chat-stream-worker-client";
import type { ChatStreamFrame } from "@/lib/assistant/chat-stream-worker-protocol";
import { applyTurnEvent, EMPTY_TURN_STATE } from "@/lib/assistant/stream";
import { WireLineFramer } from "@/lib/assistant/wire-line-framer";
import {
  parseCapturedWireLines,
  projectReplayFrames,
  type WireReplayProtocol,
} from "@/lib/assistant/wire-replay";
import { useAuthStore } from "@/stores/auth-store";
import type { AssistantWireLine } from "@/schemas/assistant-wire-log";
import type { TurnEvent, TurnReducerState } from "@/types/assistant";
import type { User } from "@/types/api";

const ACTOR_ID = "nyxid-chat-wire-replay-actor";
const TURN_ID = "turn-wire-replay";
const WORKFLOW_CONVERSATION = "chatc-1234567890abcdef1234567890abcdef";
const SCOPE_ID = "user-wire-replay";

interface ReplayCase {
  readonly name: string;
  readonly body: readonly ChatStreamFrame[];
  readonly terminal?: ChatStreamFrame | null;
  readonly preStart?: readonly ChatStreamFrame[];
  readonly afterTerminal?: readonly ChatStreamFrame[];
  readonly truncated?: boolean;
}

const textFrames: ChatStreamFrame[] = [
  {
    type: "TEXT_MESSAGE_START",
    textMessageStart: { messageId: "message-1", role: "assistant" },
  },
  {
    type: "TEXT_MESSAGE_CONTENT",
    textMessageContent: { delta: "Hello **NyxID**" },
  },
  { type: "TEXT_MESSAGE_END", textMessageEnd: { messageId: "message-1" } },
];

const cases: readonly ReplayCase[] = [
  { name: "plain text", body: textFrames },
  {
    name: "tool and run ledger",
    body: [
      {
        type: "TOOL_CALL_START",
        toolCallStart: { toolCallId: "tool-1", toolName: "catalog.list" },
      },
      {
        type: "TOOL_CALL_END",
        toolCallEnd: {
          toolCallId: "tool-1",
          status: "SUCCESS",
          result: { count: 3 },
        },
      },
    ],
  },
  {
    name: "connect card",
    body: [
      {
        type: "CUSTOM",
        custom: {
          name: "nyxid.authorization.required",
          payload: {
            reasonCode: "NYXID_SERVICE_NOT_CONNECTED",
            serviceSlug: "api-github",
            serviceLabel: "GitHub",
            safeMessage: "Connect GitHub to continue.",
          },
        },
      },
    ],
    terminal: {
      type: "RUN_FINISHED",
      runFinished: { status: "blocked" },
    },
  },
  {
    name: "action card",
    body: [
      {
        type: "CUSTOM",
        custom: {
          name: "nyxid.action.request",
          payload: {
            schemaVersion: 4,
            actorId: ACTOR_ID,
            originTurnId: TURN_ID,
            taskId: "task-1",
            stepId: "step-1",
            actionRequestId: "action-1",
            action: "service.connect",
            params: {
              catalogService: {
                serviceSlug: "api-github",
                requestedScopes: ["repo"],
              },
            },
          },
        },
      },
    ],
  },
  {
    name: "approval card",
    body: [
      {
        type: "TOOL_CALL_START",
        toolCallStart: { toolCallId: "tool-approval", toolName: "deploy" },
      },
      {
        type: "TOOL_APPROVAL_REQUEST",
        toolApprovalRequest: {
          requestId: "approval-1",
          toolCallId: "tool-approval",
          message: "Approve deployment",
          serviceSlug: "api-github",
        },
      },
    ],
  },
  {
    name: "media artifact",
    body: [
      {
        type: "MEDIA_CONTENT",
        mediaContent: {
          mediaType: "image/png",
          dataBase64: "aGVsbG8=",
          name: "chart.png",
        },
      },
    ],
  },
  { name: "completed terminal", body: [], terminal: { type: "RUN_FINISHED" } },
  {
    name: "blocked terminal",
    body: [],
    terminal: { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
  },
  {
    name: "stopped terminal",
    body: [],
    terminal: { type: "RUN_STOPPED", runStopped: { reason: "operator" } },
  },
  {
    name: "error terminal",
    body: [],
    terminal: {
      type: "RUN_ERROR",
      runError: { code: "upstream_timeout", message: "Model timed out" },
    },
  },
  {
    name: "credential-shaped error redaction",
    body: [],
    terminal: {
      type: "RUN_ERROR",
      runError: {
        code: "provider_error",
        message:
          "Authorization: Bearer secret-token and api_key=sk-secretvalue",
      },
    },
  },
  {
    name: "body-keyed frames",
    body: [
      { textMessageStart: { messageId: "body-message", role: "assistant" } },
      { textMessageContent: { delta: "Body keyed" } },
      { textMessageEnd: { messageId: "body-message" } },
    ],
    terminal: { runFinished: {} },
  },
  {
    name: "pre-start custom and post-terminal projection",
    preStart: [
      {
        custom: {
          name: "aevatar.run.context",
          payload: { actorId: "run-actor" },
        },
      },
    ],
    body: [],
    afterTerminal: [{ stateSnapshot: { version: 2 } }],
  },
  {
    name: "duplicate terminals",
    body: [],
    afterTerminal: [
      { type: "RUN_ERROR", runError: { code: "late", message: "Late" } },
    ],
  },
  {
    name: "malformed and unknown frames",
    body: [{ type: "FUTURE_FRAME", value: true }, {}],
  },
  {
    name: "truncated EOF without terminal",
    body: [
      {
        type: "TOOL_APPROVAL_REQUEST",
        toolApprovalRequest: { requestId: "approval-eof", message: "Wait" },
      },
    ],
    terminal: null,
    truncated: true,
  },
  {
    name: "EOF without terminal or approval gate",
    body: [],
    terminal: null,
  },
];

function startFrames(protocol: WireReplayProtocol): ChatStreamFrame[] {
  if (protocol === "actor") {
    return [{ type: "RUN_STARTED", turnId: TURN_ID, actorId: ACTOR_ID }];
  }
  return [
    {
      custom: {
        name: "aevatar.chat.context",
        payload: {
          scopeId: SCOPE_ID,
          conversationId: WORKFLOW_CONVERSATION,
          turnId: TURN_ID,
          stateVersion: "1",
        },
      },
    },
    { runStarted: { runId: "workflow-run-1" } },
  ];
}

function fixtureFrames(
  protocol: WireReplayProtocol,
  fixture: ReplayCase,
): ChatStreamFrame[] {
  const terminal =
    fixture.terminal === undefined
      ? { type: "RUN_FINISHED" }
      : fixture.terminal;
  return [
    ...(fixture.preStart ?? []),
    ...startFrames(protocol),
    ...fixture.body,
    ...(terminal ? [terminal] : []),
    ...(fixture.afterTerminal ?? []),
  ];
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

async function seedActor(transport: AevatarAssistantTransport): Promise<void> {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      jsonResponse({
        conversations: [
          {
            id: ACTOR_ID,
            title: "Replay parity",
            updatedAt: "2026-08-01T00:00:00.000Z",
          },
        ],
      }),
    ),
  );
  await transport.listConversations();
}

function reduce(events: readonly TurnEvent[]): TurnReducerState {
  return events.reduce(
    (state, event) => applyTurnEvent(state, event, new Date(0).toISOString()),
    EMPTY_TURN_STATE,
  );
}

function normalizeGeneratedIds(value: unknown): unknown {
  const ids = new Map<string, string>();
  const counters = new Map<string, number>();
  const normalizeString = (text: string): string => {
    const match =
      /^(?:replay-)?(assistant-activity|assistant-message|run-block|connect-card|action-card|approval-card|artifact|media|tool|step)-/.exec(
        text,
      );
    if (!match?.[1]) return text;
    const known = ids.get(text);
    if (known) return known;
    const next = (counters.get(match[1]) ?? 0) + 1;
    counters.set(match[1], next);
    const normalized = `<${match[1]}:${String(next)}>`;
    ids.set(text, normalized);
    return normalized;
  };
  const visit = (entry: unknown): unknown => {
    if (typeof entry === "string") return normalizeString(entry);
    if (Array.isArray(entry)) return entry.map(visit);
    if (entry && typeof entry === "object") {
      return Object.fromEntries(
        Object.entries(entry as Record<string, unknown>).map(([key, child]) => [
          key,
          visit(child),
        ]),
      );
    }
    return entry;
  };
  return visit(value);
}

async function projectThroughLiveTransport(
  protocol: WireReplayProtocol,
  frames: readonly ChatStreamFrame[],
): Promise<TurnEvent[]> {
  vi.spyOn(chatStreamClient, "start").mockImplementation(
    (request): ChatStreamRequestHandle => ({
      headers: Promise.resolve({
        kind: "response",
        status: 200,
        contentType: "text/event-stream",
      }),
      completion: Promise.resolve().then(() => {
        request.onFrames(frames);
        return { kind: "complete" } as const;
      }),
      cancel: vi.fn(),
    }),
  );
  const transport = new AevatarAssistantTransport();
  const conversation =
    protocol === "actor"
      ? (await seedActor(transport), { id: ACTOR_ID })
      : await transport.createConversation();
  return new Promise<TurnEvent[]>((resolve) => {
    const events: TurnEvent[] = [];
    transport.sendMessage(conversation.id, "Replay this", (event) => {
      events.push(event);
      if (event.event === "turn.completed") resolve(events);
    });
  });
}

beforeEach(() => {
  useAuthStore.getState().setUser({ id: SCOPE_ID } as User);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  useAuthStore.getState().setUser(null);
});

describe("wire replay projector parity", () => {
  const matrix = (["actor", "workflow"] as const).flatMap((protocol) =>
    cases.map((fixture) => ({ protocol, fixture })),
  );

  it.each(matrix)(
    "$protocol: $fixture.name matches live TurnEvents and reduced messages",
    async ({ protocol, fixture }) => {
      const frames = fixtureFrames(protocol, fixture);
      const liveEvents = await projectThroughLiveTransport(protocol, frames);
      const replay = projectReplayFrames(frames, {
        protocol,
        captureOutcome: "complete",
        truncated: fixture.truncated ?? false,
      });

      expect(normalizeGeneratedIds(replay.events)).toEqual(
        normalizeGeneratedIds(liveEvents),
      );
      expect(normalizeGeneratedIds(replay.state.messages)).toEqual(
        normalizeGeneratedIds(reduce(liveEvents).messages),
      );
    },
  );

  it("parses exact captured line endings through the real stream parser", () => {
    const lines: AssistantWireLine[] = [
      { text: ": keepalive", ending: "\r\n" },
      {
        text: `data: ${JSON.stringify({ type: "RUN_STARTED", turnId: TURN_ID })}`,
        ending: "\r\n",
      },
      { text: "", ending: "\r\n" },
      { text: `data: ${JSON.stringify({ type: "RUN_FINISHED" })}`, ending: "" },
    ];

    expect(parseCapturedWireLines(lines)).toEqual([
      { type: "RUN_STARTED", turnId: TURN_ID },
      { type: "RUN_FINISHED" },
    ]);
  });

  it("matches the live actor transport for the real Aevatar SSE fixture", async () => {
    const framer = new WireLineFramer(4 * 1024);
    const pushed = framer.push(new TextEncoder().encode(aevatarNyxidChatStream));
    const finished = framer.finish();
    const frames = parseCapturedWireLines([
      ...pushed.lines,
      ...finished.lines,
    ]);

    const liveEvents = await projectThroughLiveTransport("actor", frames);
    const replay = projectReplayFrames(frames, {
      protocol: "actor",
      captureOutcome: "complete",
      truncated: false,
    });

    expect(normalizeGeneratedIds(replay.events)).toEqual(
      normalizeGeneratedIds(liveEvents),
    );
    expect(normalizeGeneratedIds(replay.state.messages)).toEqual(
      normalizeGeneratedIds(reduce(liveEvents).messages),
    );
  });

  it.each([
    {
      captureOutcome: "cancelled" as const,
      status: "cancelled",
      error: null,
    },
    {
      captureOutcome: "protocol_cancel" as const,
      status: "cancelled",
      error: null,
    },
    {
      captureOutcome: "network_error" as const,
      status: "failed",
      error: { code: "network_error" },
    },
    {
      captureOutcome: "worker_error" as const,
      status: "failed",
      error: { code: "worker_error" },
    },
  ])(
    "reports terminal-free $captureOutcome captures as $status",
    ({ captureOutcome, status, error }) => {
      const projection = projectReplayFrames(
        [...startFrames("actor"), ...textFrames],
        { protocol: "actor", captureOutcome, truncated: true },
      );
      const terminal = [...projection.events].reverse().find(
        (event) => event.event === "turn.completed",
      );

      expect(terminal).toMatchObject({
        event: "turn.completed",
        status,
        error,
      });
      expect(terminal).not.toMatchObject({ status: "completed" });
    },
  );

  it("reports a terminal-free open capture as stream_closed, never completed", () => {
    const projection = projectReplayFrames(
      [...startFrames("actor"), ...textFrames],
      { protocol: "actor", truncated: true },
    );
    const terminal = [...projection.events].reverse().find(
      (event) => event.event === "turn.completed",
    );

    expect(terminal).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_closed" },
    });
    expect(terminal).not.toMatchObject({ status: "completed" });
  });

  it("keeps original frame JSON in the block source sidecar", () => {
    const sourceFrame = cases.find((fixture) => fixture.name === "connect card")
      ?.body[0];
    if (!sourceFrame) throw new Error("missing connect fixture");
    const projection = projectReplayFrames(fixtureFrames("actor", cases[2]!), {
      protocol: "actor",
      captureOutcome: "complete",
      truncated: false,
    });
    const connect = projection.state.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "connect_card");

    expect(connect).toBeDefined();
    expect(projection.sourceFramesByBlockId[connect!.block_id]).toContainEqual(
      sourceFrame,
    );
  });
});
