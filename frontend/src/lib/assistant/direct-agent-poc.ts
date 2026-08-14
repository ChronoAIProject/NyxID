import { z } from "zod";
import { isAgentPocPlanBlockId } from "@/lib/assistant/direct-agent-poc-ids";
import {
  isNyxidErrorEnvelope,
  parseJsonErrorResponse,
} from "@/lib/assistant/direct-http-error";
import { drainSseBuffer, flushSseBuffer } from "@/lib/assistant/sse";
import { useAuthStore } from "@/stores/auth-store";
import type {
  AssistantMessage,
  RunContentBlock,
  RunStepStatus,
  TextContentBlock,
  TurnEvent,
} from "@/types/assistant";

export const DIRECT_AGENT_POC_URL = "/api/v1/assistant/direct/agent";
export const MAX_AGENT_POC_CONTENT_BYTES = 128 * 1024;
export const MAX_AGENT_POC_RESULT_PREVIEW_CHARS = 2 * 1024;

const stageNameSchema = z.enum(["understand", "plan", "execute", "final"]);
const frameSchemas = {
  "run.started": z
    .object({
      type: z.literal("run.started"),
      run_id: z.string().min(1),
      model: z.string().min(1),
      skill_slug: z.string().nullable(),
      effort: z.string().nullable(),
      limits: z
        .object({
          max_tool_calls: z.number().int().nonnegative(),
          max_llm_calls: z.number().int().nonnegative(),
          deadline_ms: z.number().int().positive(),
        })
        .strict(),
    })
    .strict(),
  stage: z
    .object({
      type: z.literal("stage"),
      stage: stageNameSchema,
      status: z.enum(["started", "completed"]),
      detail: z.string(),
    })
    .strict(),
  "text.delta": z
    .object({
      type: z.literal("text.delta"),
      stage: z.enum(["plan", "final"]),
      text: z.string(),
    })
    .strict(),
  "tool.started": z
    .object({
      type: z.literal("tool.started"),
      call_id: z.string().min(1),
      index: z.number().int().nonnegative(),
      tool: z.string().min(1),
      target: z
        .object({
          service_slug: z.string().min(1),
          endpoint: z.string().min(1),
        })
        .strict(),
    })
    .strict(),
  "tool.completed": z
    .object({
      type: z.literal("tool.completed"),
      call_id: z.string().min(1),
      tool: z.string().min(1),
      outcome: z.enum([
        "ok",
        "failed",
        "skipped",
        "denied",
        "outcome_uncertain",
      ]),
      status: z.number().int().nonnegative(),
      duration_ms: z.number().int().nonnegative(),
      result_bytes: z.number().int().nonnegative(),
      truncated: z.boolean(),
      result_preview: z
        .string()
        .max(MAX_AGENT_POC_RESULT_PREVIEW_CHARS)
        .optional(),
    })
    .strict(),
  error: z
    .object({
      type: z.literal("error"),
      code: z.enum([
        "deadline_exceeded",
        "upstream_failed",
        "internal",
        "context_overflow",
      ]),
      message: z.string(),
    })
    .strict(),
  done: z
    .object({
      type: z.literal("done"),
      status: z.literal("completed"),
      finish_reason: z.string().min(1),
      tool_calls: z.number().int().nonnegative(),
      llm_calls: z.number().int().nonnegative(),
      duration_ms: z.number().int().nonnegative(),
    })
    .strict(),
} as const;

type KnownFrameType = keyof typeof frameSchemas;
type AgentFrame = z.infer<(typeof frameSchemas)[KnownFrameType]>;
type StageName = z.infer<typeof stageNameSchema>;
type TerminalStatus = "completed" | "failed" | "cancelled";
const ORDERED_STAGES: readonly StageName[] = [
  "understand",
  "plan",
  "execute",
  "final",
];

export interface DirectAgentPocMessage {
  readonly role: "user" | "assistant";
  readonly content: string;
}

export interface DirectAgentPocRequest {
  readonly messages: readonly DirectAgentPocMessage[];
  readonly model: string;
  readonly skill_slug?: string;
  readonly effort?: string;
}

interface ServerTerminal {
  readonly status: "completed" | "failed";
  readonly error: { readonly code: string; readonly message: string } | null;
}

export interface AgentPocRuntime {
  readonly turnId: string;
  readonly controller: AbortController;
  readonly messageId: string;
  readonly runBlockId: string;
  runBlock: RunContentBlock | null;
  stageIndexes: Map<StageName, number>;
  toolCalls: Map<string, { readonly index: number; readonly tool: string }>;
  activeStage: StageName | null;
  completedStageCount: number;
  planText: string;
  finalText: string;
  planOpened: boolean;
  planClosed: boolean;
  finalOpened: boolean;
  finalClosed: boolean;
  messageStarted: boolean;
  messageClosed: boolean;
  sawRunStarted: boolean;
  sawServerTerminal: boolean;
  sawDoneMarker: boolean;
  serverTerminal: ServerTerminal | null;
  settled: boolean;
}

export interface AgentPocStreamOptions {
  readonly request: DirectAgentPocRequest;
  readonly fetch: typeof fetch;
  readonly firstByteTimeoutMs: number;
  readonly idleTimeoutMs: number;
  readonly now: () => number;
  readonly nextCursor: () => number;
  readonly emit: (event: TurnEvent) => void;
  readonly finishDrain: () => void;
}

class AgentPocTimeoutError extends Error {
  readonly code: "first_byte_timeout" | "idle_timeout";

  constructor(code: "first_byte_timeout" | "idle_timeout") {
    super(
      code === "first_byte_timeout"
        ? "The agent did not start replying in time."
        : "The agent stream stopped responding.",
    );
    this.code = code;
  }
}

export function createAgentPocRuntime(
  turnId: string,
  controller: AbortController,
): AgentPocRuntime {
  const messageId = `${turnId}-agent-poc-message`;
  return {
    turnId,
    controller,
    messageId,
    runBlockId: `${messageId}-run`,
    runBlock: null,
    stageIndexes: new Map(),
    toolCalls: new Map(),
    activeStage: null,
    completedStageCount: 0,
    planText: "",
    finalText: "",
    planOpened: false,
    planClosed: false,
    finalOpened: false,
    finalClosed: false,
    messageStarted: false,
    messageClosed: false,
    sawRunStarted: false,
    sawServerTerminal: false,
    sawDoneMarker: false,
    serverTerminal: null,
    settled: false,
  };
}

export function projectAgentPocMessages(
  messages: readonly AssistantMessage[],
): DirectAgentPocMessage[] {
  const payload: DirectAgentPocMessage[] = [];
  for (const message of messages) {
    if (message.role !== "user" && message.role !== "assistant") continue;
    const content = message.blocks
      .filter(
        (block): block is TextContentBlock =>
          block.type === "text" && !isAgentPocPlanBlockId(block.block_id),
      )
      .map((block) => block.text)
      .filter(Boolean)
      .join("\n\n");
    if (content) payload.push({ role: message.role, content });
  }
  return payload;
}

export async function streamAgentPocTurn(
  runtime: AgentPocRuntime,
  options: AgentPocStreamOptions,
): Promise<void> {
  const firstByteDeadline = options.now() + options.firstByteTimeoutMs;
  try {
    const response = await withTimeout(
      options.fetch(DIRECT_AGENT_POC_URL, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "text/event-stream",
        },
        credentials: "include",
        body: JSON.stringify(options.request),
        signal: runtime.controller.signal,
      }),
      options.firstByteTimeoutMs,
      runtime,
      "first_byte_timeout",
    );
    if (!response.ok) {
      const body = await parseJsonErrorResponse(response);
      const nyxidError = isNyxidErrorEnvelope(body) ? body : null;
      settle(runtime, options, "failed", {
        code: `http_${String(response.status)}`,
        message: nyxidError?.message ?? "The agent stream could not be started.",
      });
      // The terminal event must be visible before the identity transition
      // aborts and discards active Assistant transports.
      if (response.status === 401 && nyxidError) {
        queueMicrotask(() => useAuthStore.getState().setUser(null));
      }
      return;
    }
    if (!response.body) {
      settle(runtime, options, "failed", {
        code: "missing_stream",
        message: "The agent returned no stream.",
      });
      return;
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let firstRead = true;
    let stopReading = false;
    while (!stopReading) {
      const timeout = firstRead
        ? Math.max(0, firstByteDeadline - options.now())
        : options.idleTimeoutMs;
      const result = await withTimeout(
        reader.read(),
        timeout,
        runtime,
        firstRead ? "first_byte_timeout" : "idle_timeout",
      );
      firstRead = false;
      if (result.done) break;
      buffer += decoder.decode(result.value, { stream: true });
      const drained = drainSseBuffer(buffer);
      buffer = drained.rest;
      for (const payload of drained.payloads) {
        if (handlePayload(runtime, options, payload)) {
          stopReading = true;
          break;
        }
      }
    }

    if (!stopReading) {
      buffer += decoder.decode();
      for (const payload of flushSseBuffer(buffer)) {
        if (handlePayload(runtime, options, payload)) break;
      }
    }
    if (!runtime.settled) {
      settle(runtime, options, "failed", {
        code: "truncated_stream",
        message: "The agent stream ended before its terminal marker.",
      });
    }
  } catch (error) {
    if (!runtime.settled) {
      const timeout = error instanceof AgentPocTimeoutError ? error : null;
      settle(runtime, options, "failed", {
        code: timeout?.code ?? "network_error",
        message:
          timeout?.message ??
          (error instanceof Error ? error.message : "The agent stream failed."),
      });
    }
  } finally {
    options.finishDrain();
  }
}

export function cancelAgentPocTurn(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit" | "finishDrain">,
): void {
  if (runtime.settled) return;
  runtime.controller.abort();
  settle(runtime, options, "cancelled", null);
  options.finishDrain();
}

async function withTimeout<T>(
  task: Promise<T>,
  timeoutMs: number,
  runtime: AgentPocRuntime,
  code: "first_byte_timeout" | "idle_timeout",
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      task,
      new Promise<T>((_resolve, reject) => {
        timer = setTimeout(
          () => {
            reject(new AgentPocTimeoutError(code));
            runtime.controller.abort();
          },
          Math.max(0, timeoutMs),
        );
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

function handlePayload(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  payload: string,
): boolean {
  if (payload === "[DONE]") {
    if (runtime.sawDoneMarker) {
      protocolError(runtime, options);
      return true;
    }
    runtime.sawDoneMarker = true;
    if (!runtime.serverTerminal) {
      settle(runtime, options, "failed", {
        code: "truncated_stream",
        message: "The agent stream ended without a server terminal frame.",
      });
      return true;
    }
    settle(
      runtime,
      options,
      runtime.serverTerminal.status,
      runtime.serverTerminal.error,
    );
    return true;
  }

  let value: unknown;
  try {
    value = JSON.parse(payload) as unknown;
  } catch {
    protocolError(runtime, options);
    return true;
  }
  if (!value || typeof value !== "object" || !("type" in value)) {
    protocolError(runtime, options);
    return true;
  }
  const type = (value as { type?: unknown }).type;
  if (typeof type !== "string") {
    protocolError(runtime, options);
    return true;
  }
  if (!Object.hasOwn(frameSchemas, type)) return false;
  const schema = frameSchemas[type as KnownFrameType];
  const parsed = schema.safeParse(value);
  if (!parsed.success || runtime.sawServerTerminal) {
    protocolError(runtime, options);
    return true;
  }
  if (!runtime.sawRunStarted && type !== "run.started") {
    protocolError(runtime, options);
    return true;
  }
  applyFrame(runtime, options, parsed.data as AgentFrame);
  return runtime.settled;
}

function applyFrame(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  frame: AgentFrame,
): void {
  switch (frame.type) {
    case "run.started":
      if (runtime.sawRunStarted) return protocolError(runtime, options);
      runtime.sawRunStarted = true;
      openRun(runtime, options);
      return;
    case "stage":
      applyStage(runtime, options, frame);
      return;
    case "text.delta":
      appendText(runtime, options, frame.stage, frame.text);
      return;
    case "tool.started":
      startTool(runtime, options, frame);
      return;
    case "tool.completed":
      completeTool(runtime, options, frame);
      return;
    case "error":
      runtime.sawServerTerminal = true;
      runtime.serverTerminal = {
        status: "failed",
        error: { code: frame.code, message: frame.message },
      };
      return;
    case "done":
      if (!isSuccessfulTerminalComplete(runtime)) {
        protocolError(runtime, options);
        return;
      }
      runtime.sawServerTerminal = true;
      runtime.serverTerminal = { status: "completed", error: null };
      return;
  }
}

function openRun(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
): void {
  runtime.messageStarted = true;
  runtime.runBlock = {
    type: "run",
    block_id: runtime.runBlockId,
    title: "Agent run",
    steps_total: 0,
    steps_complete: 0,
    state: "running",
    steps: [],
  };
  emit(options, {
    event: "message.started",
    message_id: runtime.messageId,
    role: "assistant",
  });
  emit(options, {
    event: "block.started",
    message_id: runtime.messageId,
    block_id: runtime.runBlockId,
    index: 0,
    block: runtime.runBlock,
  });
}

function applyStage(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  frame: z.infer<(typeof frameSchemas)["stage"]>,
): void {
  const existing = runtime.stageIndexes.get(frame.stage);
  if (frame.status === "started") {
    if (
      existing !== undefined ||
      runtime.activeStage !== null ||
      ORDERED_STAGES[runtime.completedStageCount] !== frame.stage
    ) {
      return protocolError(runtime, options);
    }
    const index = appendStep(runtime, {
      status: "active",
      label: stageLabel(frame.stage),
      meta: frame.detail,
      service_slug: null,
    });
    runtime.stageIndexes.set(frame.stage, index);
    runtime.activeStage = frame.stage;
    updateRun(runtime, options);
    return;
  }
  if (
    existing === undefined ||
    runtime.activeStage !== frame.stage ||
    (frame.stage === "execute" && hasActiveTool(runtime)) ||
    runtime.runBlock?.steps[existing]?.status !== "active"
  ) {
    return protocolError(runtime, options);
  }
  setStep(runtime, existing, "done", frame.detail);
  runtime.activeStage = null;
  runtime.completedStageCount += 1;
  updateRun(runtime, options);
  if (frame.stage === "plan") closeText(runtime, options, "plan");
  if (frame.stage === "final") closeText(runtime, options, "final");
}

function appendText(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  stage: "plan" | "final",
  text: string,
): void {
  const isPlan = stage === "plan";
  const opened = isPlan ? runtime.planOpened : runtime.finalOpened;
  const closed = isPlan ? runtime.planClosed : runtime.finalClosed;
  if (closed || runtime.activeStage !== stage) {
    return protocolError(runtime, options);
  }
  if (!text) return;
  const blockId = isPlan
    ? `${runtime.messageId}-agent-poc-plan`
    : `${runtime.messageId}-final`;
  if (!opened) {
    if (isPlan) runtime.planOpened = true;
    else runtime.finalOpened = true;
    emit(options, {
      event: "block.started",
      message_id: runtime.messageId,
      block_id: blockId,
      index: isPlan ? 1 : 2,
      block: { type: "text", block_id: blockId, text: "" },
    });
  }
  if (isPlan) runtime.planText += text;
  else runtime.finalText += text;
  emit(options, { event: "block.delta", block_id: blockId, text });
}

function closeText(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  stage: "plan" | "final",
): void {
  const isPlan = stage === "plan";
  if (
    isPlan
      ? !runtime.planOpened || runtime.planClosed
      : !runtime.finalOpened || runtime.finalClosed
  ) {
    return;
  }
  const blockId = isPlan
    ? `${runtime.messageId}-agent-poc-plan`
    : `${runtime.messageId}-final`;
  const text = isPlan ? runtime.planText : runtime.finalText;
  if (isPlan) runtime.planClosed = true;
  else runtime.finalClosed = true;
  emit(options, {
    event: "block.completed",
    block_id: blockId,
    block: { type: "text", block_id: blockId, text },
  });
}

function startTool(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  frame: z.infer<(typeof frameSchemas)["tool.started"]>,
): void {
  if (
    runtime.activeStage !== "execute" ||
    runtime.toolCalls.has(frame.call_id)
  )
    return protocolError(runtime, options);
  const index = appendStep(runtime, {
    status: "active",
    label: `${frame.target.service_slug} · ${frame.target.endpoint}`,
    meta: frame.tool,
    service_slug: frame.target.service_slug,
  });
  runtime.toolCalls.set(frame.call_id, { index, tool: frame.tool });
  updateRun(runtime, options);
}

function completeTool(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  frame: z.infer<(typeof frameSchemas)["tool.completed"]>,
): void {
  const started = runtime.toolCalls.get(frame.call_id);
  const step = started
    ? runtime.runBlock?.steps[started.index]
    : undefined;
  if (
    runtime.activeStage !== "execute" ||
    !started ||
    started.tool !== frame.tool ||
    !step ||
    step.status !== "active"
  ) {
    return protocolError(runtime, options);
  }
  const status: RunStepStatus =
    frame.outcome === "ok"
      ? "done"
      : frame.outcome === "skipped"
        ? "skipped"
        : "failed";
  const meta =
    frame.outcome === "outcome_uncertain"
      ? "outcome uncertain"
      : `${frame.tool} · HTTP ${String(frame.status)}`;
  setStep(runtime, started.index, status, meta, frame.result_preview ?? null);
  updateRun(runtime, options);
}

function hasActiveTool(runtime: AgentPocRuntime): boolean {
  return [...runtime.toolCalls.values()].some(
    ({ index }) => runtime.runBlock?.steps[index]?.status === "active",
  );
}

function isSuccessfulTerminalComplete(runtime: AgentPocRuntime): boolean {
  return (
    runtime.completedStageCount === ORDERED_STAGES.length &&
    runtime.activeStage === null &&
    !hasActiveTool(runtime) &&
    runtime.runBlock?.steps.every(
      (step) => step.status !== "active" && step.status !== "waiting",
    ) === true
  );
}

function appendStep(
  runtime: AgentPocRuntime,
  step: Pick<
    RunContentBlock["steps"][number],
    "status" | "label" | "meta" | "service_slug"
  >,
): number {
  if (!runtime.runBlock) throw new Error("Agent run block is not open.");
  const index = runtime.runBlock.steps.length;
  runtime.runBlock = {
    ...runtime.runBlock,
    steps: [
      ...runtime.runBlock.steps,
      {
        index,
        ...step,
        result_preview: null,
        artifact_id: null,
        approval_request_id: null,
      },
    ],
  };
  return index;
}

function setStep(
  runtime: AgentPocRuntime,
  index: number,
  status: RunStepStatus,
  meta: string,
  resultPreview?: string | null,
): void {
  if (!runtime.runBlock) return;
  runtime.runBlock = {
    ...runtime.runBlock,
    steps: runtime.runBlock.steps.map((step) =>
      step.index === index
        ? {
            ...step,
            status,
            meta,
            result_preview:
              resultPreview === undefined
                ? step.result_preview
                : resultPreview,
          }
        : step,
    ),
  };
}

function updateRun(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
): void {
  if (!runtime.runBlock) return;
  runtime.runBlock = withRunCounts(runtime.runBlock);
  emit(options, {
    event: "block.updated",
    block_id: runtime.runBlockId,
    patch: runtime.runBlock,
  });
}

function settle(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  status: TerminalStatus,
  error: { readonly code: string; readonly message: string } | null,
): void {
  if (runtime.settled) return;
  runtime.settled = true;
  closeText(runtime, options, "plan");
  closeText(runtime, options, "final");
  if (runtime.runBlock) {
    const stepStatus: RunStepStatus =
      status === "completed"
        ? "done"
        : status === "failed"
          ? "failed"
          : "skipped";
    runtime.runBlock = withRunCounts({
      ...runtime.runBlock,
      state: status,
      steps: runtime.runBlock.steps.map((step) =>
        step.status === "active" || step.status === "waiting"
          ? { ...step, status: stepStatus }
          : step,
      ),
    });
    emit(options, {
      event: "block.completed",
      block_id: runtime.runBlockId,
      block: runtime.runBlock,
    });
  }
  if (runtime.messageStarted && !runtime.messageClosed) {
    runtime.messageClosed = true;
    emit(options, {
      event: "message.completed",
      message_id: runtime.messageId,
    });
  }
  emit(options, {
    event: "turn.completed",
    turn_id: runtime.turnId,
    status,
    error,
  });
}

function protocolError(
  runtime: AgentPocRuntime,
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
): void {
  runtime.controller.abort();
  settle(runtime, options, "failed", {
    code: "protocol_error",
    message: "The agent stream violated its frame contract.",
  });
}

function withRunCounts(block: RunContentBlock): RunContentBlock {
  return {
    ...block,
    steps_total: block.steps.length,
    steps_complete: block.steps.filter((step) => step.status === "done").length,
  };
}

function stageLabel(stage: StageName): string {
  if (stage === "final") return "Report";
  return stage[0]!.toUpperCase() + stage.slice(1);
}

function emit(
  options: Pick<AgentPocStreamOptions, "nextCursor" | "emit">,
  event: Record<string, unknown> & { readonly event: TurnEvent["event"] },
): void {
  options.emit({ ...event, cursor: options.nextCursor() } as TurnEvent);
}
