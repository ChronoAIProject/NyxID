import {
  AGUIEventType,
  CustomEventName,
  parseCustomEvent,
  type AGUIEvent,
} from "@/lib/assistant/agui-types";
import {
  parseRunContextData,
  parseHumanInputRequestData,
  parseStepCompletedData,
  parseStepRequestData,
  parseWaitingSignalData,
} from "@/lib/assistant/custom-event-data";
import { normalizeBackendSseFrame } from "@/lib/assistant/sse-frame-normalizer";

type JsonRecord = Record<string, unknown>;
type RuntimeFinalOutputSource =
  | "run_finished"
  | "step_completed"
  | "text_message_end";

const FINAL_OUTPUT_SOURCE_PRIORITY: Record<RuntimeFinalOutputSource, number> = {
  text_message_end: 1,
  step_completed: 2,
  run_finished: 3,
};

function asRecord(value: unknown): JsonRecord | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }

  return value as JsonRecord;
}

function readOptionalString(
  record: JsonRecord | undefined,
  ...keys: string[]
): string {
  if (!record) {
    return "";
  }

  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) {
      return value;
    }
  }

  return "";
}

export type RuntimeEvent = AGUIEvent;

export type RuntimeStepInfo = {
  id?: string;
  name: string;
  status: "running" | "done" | "error";
  startedAt: number;
  finishedAt?: number;
  output?: string;
  error?: string;
  stepType?: string;
};

export type RuntimeToolCallInfo = {
  id: string;
  name: string;
  status: "running" | "done" | "error";
  startedAt: number;
  finishedAt?: number;
  result?: string;
  error?: string;
};

export type RuntimeToolApprovalRequestInfo = {
  requestId: string;
  toolName: string;
  toolCallId: string;
  argumentsJson: string;
  isDestructive: boolean;
  timeoutSeconds: number;
};

export type RuntimeRunInterventionInfo = {
  key: string;
  kind: "human_input" | "human_approval" | "wait_signal";
  actorId?: string;
  prompt: string;
  runId: string;
  signalName?: string;
  stepId: string;
  timeoutSeconds?: number;
  variableName?: string;
};

export type RuntimeEventAccumulator = {
  actorId: string;
  assistantText: string;
  commandId: string;
  correlationId: string;
  errorCode: string;
  errorText: string;
  events: RuntimeEvent[];
  finalOutput: string;
  finalOutputSource?: RuntimeFinalOutputSource;
  pendingApproval?: RuntimeToolApprovalRequestInfo;
  pendingRunIntervention?: RuntimeRunInterventionInfo;
  runId: string;
  steps: RuntimeStepInfo[];
  thinking: string;
  toolCalls: RuntimeToolCallInfo[];
};

export function normalizeRuntimeFrame(raw: unknown): RuntimeEvent | null {
  return normalizeBackendSseFrame(raw);
}

function setFinalOutput(
  accumulator: RuntimeEventAccumulator,
  output: string | null | undefined,
  source: RuntimeFinalOutputSource,
): void {
  const finalOutput = String(output || "").trim();
  if (!finalOutput) {
    return;
  }

  const currentPriority = accumulator.finalOutputSource
    ? FINAL_OUTPUT_SOURCE_PRIORITY[accumulator.finalOutputSource]
    : 0;
  const nextPriority = FINAL_OUTPUT_SOURCE_PRIORITY[source];

  if (nextPriority >= currentPriority) {
    accumulator.finalOutput = finalOutput;
    accumulator.finalOutputSource = source;
  }
}

export function describeRuntimeEvent(event: RuntimeEvent): string {
  if (event.type !== AGUIEventType.CUSTOM) {
    return event.type;
  }

  return `CUSTOM · ${parseCustomEvent(event).name}`;
}

export function extractRunContext(
  event: RuntimeEvent,
): { actorId?: string; commandId?: string; correlationId?: string } | null {
  if (event.type !== AGUIEventType.CUSTOM) {
    return null;
  }

  const custom = parseCustomEvent(event);
  if (
    custom.name !== CustomEventName.RunContext &&
    custom.name !== "aevatar.run.context"
  ) {
    return null;
  }

  const data = parseRunContextData(custom.data);
  if (!data) {
    return null;
  }

  return {
    actorId: data.actorId,
    commandId: data.commandId,
    correlationId: data.correlationId,
  };
}

export function extractStepCompleted(
  event: RuntimeEvent,
): {
  error?: string;
  output?: string;
  stepId: string;
  success?: boolean;
} | null {
  if (event.type !== AGUIEventType.CUSTOM) {
    return null;
  }

  const custom = parseCustomEvent(event);
  if (
    custom.name !== CustomEventName.StepCompleted &&
    custom.name !== "aevatar.step.completed"
  ) {
    return null;
  }

  const data = parseStepCompletedData(custom.data);
  const stepId = data?.stepId?.trim() || "";
  if (!stepId) {
    return null;
  }

  return {
    error: data?.error,
    output: data?.output,
    stepId,
    success: data?.success,
  };
}

export function extractStepCompletedOutput(event: RuntimeEvent): string | null {
  return extractStepCompleted(event)?.output ?? null;
}

export function extractRunFinishedOutput(event: RuntimeEvent): string | null {
  if (event.type !== AGUIEventType.RUN_FINISHED) {
    return null;
  }

  const result = (event as unknown as JsonRecord).result;
  if (typeof result === "string") {
    return result.trim() || null;
  }

  const record = asRecord(result);
  // Refactor (iter98/cluster-790): Old: backend could synthesize missed-live text frames. New: consumer fallback reads typed GAgentDraftRunResultPayload.output from RunFinished.result.
  const candidate = readOptionalString(
    record,
    "output",
    "Output",
    "message",
    "text",
  );
  return candidate || null;
}

export function extractReasoningDelta(event: RuntimeEvent): string | null {
  if (event.type !== AGUIEventType.CUSTOM) {
    return null;
  }

  const custom = parseCustomEvent(event);
  if (custom.name !== "aevatar.llm.reasoning") {
    return null;
  }

  const payload = asRecord(custom.data);
  const delta = readOptionalString(payload, "delta", "Delta");
  return delta || null;
}

export function extractStepRequest(
  event: RuntimeEvent,
): { input: string; stepId: string; stepType: string } | null {
  if (event.type !== AGUIEventType.CUSTOM) {
    return null;
  }

  const custom = parseCustomEvent(event);
  if (
    custom.name !== CustomEventName.StepRequest &&
    custom.name !== "aevatar.step.request"
  ) {
    return null;
  }

  const data = parseStepRequestData(custom.data);
  if (!data?.stepId?.trim() && !data?.stepType?.trim()) {
    return null;
  }

  return {
    input: data?.input?.trim() || "",
    stepId: data?.stepId?.trim() || "",
    stepType: data?.stepType?.trim() || "",
  };
}

export function extractToolApprovalRequest(
  event: RuntimeEvent,
): RuntimeToolApprovalRequestInfo | null {
  if (String(event.type) === "TOOL_APPROVAL_REQUEST") {
    const requestId = readOptionalString(
      event as unknown as JsonRecord,
      "requestId",
      "request_id",
    );
    if (!requestId) {
      return null;
    }

    return {
      argumentsJson: readOptionalString(
        event as unknown as JsonRecord,
        "argumentsJson",
        "arguments_json",
      ),
      isDestructive: Boolean(
        (event as unknown as JsonRecord).isDestructive ??
        (event as unknown as JsonRecord).is_destructive,
      ),
      requestId,
      timeoutSeconds:
        typeof (event as unknown as JsonRecord).timeoutSeconds === "number"
          ? ((event as unknown as JsonRecord).timeoutSeconds as number)
          : typeof (event as unknown as JsonRecord).timeout_seconds === "number"
            ? ((event as unknown as JsonRecord).timeout_seconds as number)
            : 15,
      toolCallId: readOptionalString(
        event as unknown as JsonRecord,
        "toolCallId",
        "tool_call_id",
      ),
      toolName: readOptionalString(
        event as unknown as JsonRecord,
        "toolName",
        "tool_name",
      ),
    };
  }

  if (event.type !== AGUIEventType.CUSTOM) {
    return null;
  }

  const custom = parseCustomEvent(event);
  if (custom.name !== "TOOL_APPROVAL_REQUEST") {
    return null;
  }

  const raw = asRecord(custom.data);
  const payload = asRecord(raw?.value) ?? raw;
  if (!payload) {
    return null;
  }

  const requestId = readOptionalString(payload, "requestId", "request_id");
  if (!requestId) {
    return null;
  }

  return {
    argumentsJson: readOptionalString(
      payload,
      "argumentsJson",
      "arguments_json",
    ),
    isDestructive: Boolean(
      payload.isDestructive ?? payload.is_destructive ?? false,
    ),
    requestId,
    timeoutSeconds:
      typeof payload.timeoutSeconds === "number"
        ? payload.timeoutSeconds
        : typeof payload.timeout_seconds === "number"
          ? payload.timeout_seconds
          : 15,
    toolCallId: readOptionalString(payload, "toolCallId", "tool_call_id"),
    toolName: readOptionalString(payload, "toolName", "tool_name"),
  };
}

export function isRawObserved(event: RuntimeEvent): boolean {
  if (event.type !== AGUIEventType.CUSTOM) {
    return false;
  }

  return parseCustomEvent(event).name === "aevatar.raw.observed";
}

function isHumanApprovalSuspension(value: string): boolean {
  const normalized = value.trim().toLowerCase();
  return normalized.includes("approval") || normalized.includes("approve");
}

function buildRunInterventionKey(
  kind: RuntimeRunInterventionInfo["kind"],
  runId: string,
  stepId: string,
  signalName?: string,
): string {
  return [kind, runId || "run", stepId || "step", signalName || ""].join(":");
}

function buildHumanInputIntervention(
  data: ReturnType<typeof parseHumanInputRequestData>,
  actorId?: string,
): RuntimeRunInterventionInfo | null {
  const stepId = String(data?.stepId || "").trim();
  if (!stepId) {
    return null;
  }

  const runId = String(data?.runId || "").trim();
  const suspensionType = String(data?.suspensionType || "").trim();
  const kind = isHumanApprovalSuspension(suspensionType)
    ? "human_approval"
    : "human_input";
  const metadata = data?.metadata ?? {};
  const variableName =
    metadata.variableName ||
    metadata.variable_name ||
    metadata.assignedVariable ||
    metadata.assigned_variable ||
    undefined;

  return {
    actorId: actorId?.trim() || undefined,
    key: buildRunInterventionKey(kind, runId, stepId),
    kind,
    prompt:
      String(data?.prompt || "").trim() ||
      (kind === "human_approval"
        ? "This run is waiting for approval."
        : "This run is waiting for additional input."),
    runId,
    stepId,
    timeoutSeconds:
      typeof data?.timeoutSeconds === "number" && data.timeoutSeconds > 0
        ? data.timeoutSeconds
        : undefined,
    variableName,
  };
}

function buildWaitingSignalIntervention(
  data: ReturnType<typeof parseWaitingSignalData>,
  actorId?: string,
): RuntimeRunInterventionInfo | null {
  const stepId = String(data?.stepId || "").trim();
  if (!stepId) {
    return null;
  }

  const runId = String(data?.runId || "").trim();
  const signalName = String(data?.signalName || "").trim() || "continue";
  return {
    actorId: actorId?.trim() || undefined,
    key: buildRunInterventionKey("wait_signal", runId, stepId, signalName),
    kind: "wait_signal",
    prompt:
      String(data?.prompt || "").trim() ||
      `Runtime is waiting for signal ${signalName}.`,
    runId,
    signalName,
    stepId,
    timeoutSeconds:
      typeof data?.timeoutMs === "number" && data.timeoutMs > 0
        ? Math.max(1, Math.round(data.timeoutMs / 1000))
        : undefined,
  };
}

export function extractRunInterventionRequest(
  event: RuntimeEvent,
  actorId?: string,
): RuntimeRunInterventionInfo | null {
  if (event.type === AGUIEventType.HUMAN_INPUT_REQUEST) {
    return buildHumanInputIntervention(
      parseHumanInputRequestData(event as unknown as JsonRecord),
      actorId,
    );
  }

  if (event.type !== AGUIEventType.CUSTOM) {
    return null;
  }

  const custom = parseCustomEvent(event);
  if (custom.name === "aevatar.human_input.request") {
    return buildHumanInputIntervention(
      parseHumanInputRequestData(custom.data),
      actorId,
    );
  }

  if (custom.name === CustomEventName.WaitingSignal) {
    return buildWaitingSignalIntervention(
      parseWaitingSignalData(custom.data),
      actorId,
    );
  }

  return null;
}

export function createRuntimeEventAccumulator(input?: {
  actorId?: string;
}): RuntimeEventAccumulator {
  return {
    actorId: input?.actorId ?? "",
    assistantText: "",
    commandId: "",
    correlationId: "",
    errorCode: "",
    errorText: "",
    events: [],
    finalOutput: "",
    pendingApproval: undefined,
    pendingRunIntervention: undefined,
    runId: "",
    steps: [],
    thinking: "",
    toolCalls: [],
  };
}

export function applyRuntimeEvent(
  accumulator: RuntimeEventAccumulator,
  event: RuntimeEvent,
): RuntimeEventAccumulator {
  accumulator.events.push(event);

  if (
    event.type === AGUIEventType.RUN_FINISHED ||
    event.type === AGUIEventType.RUN_ERROR
  ) {
    accumulator.pendingRunIntervention = undefined;
  }

  if (event.type === AGUIEventType.RUN_STARTED) {
    accumulator.actorId =
      readOptionalString(
        event as unknown as JsonRecord,
        "actorId",
        "threadId",
      ) || accumulator.actorId;
    accumulator.commandId =
      readOptionalString(
        event as unknown as JsonRecord,
        "commandId",
        "command_id",
      ) || accumulator.commandId;
    accumulator.correlationId =
      readOptionalString(
        event as unknown as JsonRecord,
        "correlationId",
        "correlation_id",
      ) || accumulator.correlationId;
    accumulator.runId = event.runId || accumulator.runId;
  }

  if (event.type === AGUIEventType.TEXT_MESSAGE_CONTENT) {
    accumulator.assistantText += String(event.delta || "");
  }

  if (event.type === AGUIEventType.TEXT_MESSAGE_END) {
    const finalText = String(
      (event as unknown as JsonRecord).message ||
        (event as unknown as JsonRecord).delta ||
        "",
    ).trim();
    setFinalOutput(accumulator, finalText, "text_message_end");
  }

  if (event.type === AGUIEventType.STEP_STARTED) {
    accumulator.pendingRunIntervention = undefined;
    const stepName =
      String(event.stepName || "").trim() ||
      `Step ${accumulator.steps.length + 1}`;
    accumulator.steps.push({
      id: stepName,
      name: stepName,
      startedAt: event.timestamp || Date.now(),
      status: "running",
    });
  }

  if (event.type === AGUIEventType.STEP_FINISHED) {
    accumulator.pendingRunIntervention = undefined;
    const stepName = String(event.stepName || "").trim();
    const existingStep = accumulator.steps.find(
      (step) =>
        step.status === "running" &&
        (!stepName || step.name === stepName || step.id === stepName),
    );
    if (existingStep) {
      existingStep.finishedAt = event.timestamp || Date.now();
      existingStep.status = "done";
    }
  }

  if (event.type === AGUIEventType.TOOL_CALL_START) {
    const toolName = String(event.toolName || "").trim() || "Tool";
    const toolId =
      String(event.toolCallId || "").trim() ||
      `${toolName}-${accumulator.toolCalls.length + 1}`;
    accumulator.toolCalls.push({
      id: toolId,
      name: toolName,
      startedAt: event.timestamp || Date.now(),
      status: "running",
    });
  }

  if (event.type === AGUIEventType.TOOL_CALL_END) {
    const toolId = String(event.toolCallId || "").trim();
    const existingTool = accumulator.toolCalls.find(
      (tool) => tool.status === "running" && (!toolId || tool.id === toolId),
    );
    if (existingTool) {
      existingTool.finishedAt = event.timestamp || Date.now();
      existingTool.result =
        "result" in event && typeof event.result === "string"
          ? event.result.trim()
          : "";
      existingTool.status = "done";
    }
  }

  if (event.type === AGUIEventType.RUN_ERROR) {
    accumulator.commandId =
      readOptionalString(
        event as unknown as JsonRecord,
        "commandId",
        "command_id",
      ) || accumulator.commandId;
    accumulator.correlationId =
      readOptionalString(
        event as unknown as JsonRecord,
        "correlationId",
        "correlation_id",
      ) || accumulator.correlationId;
    accumulator.errorCode =
      readOptionalString(
        event as unknown as JsonRecord,
        "code",
        "errorCode",
        "error_code",
      ) || accumulator.errorCode;
    accumulator.errorText = String(
      event.message || "Assistant run failed.",
    ).trim();
  }

  if (event.type === AGUIEventType.RUN_FINISHED) {
    accumulator.commandId =
      readOptionalString(
        event as unknown as JsonRecord,
        "commandId",
        "command_id",
      ) || accumulator.commandId;
    accumulator.correlationId =
      readOptionalString(
        event as unknown as JsonRecord,
        "correlationId",
        "correlation_id",
      ) || accumulator.correlationId;
    const finalOutput = extractRunFinishedOutput(event);
    setFinalOutput(accumulator, finalOutput, "run_finished");
  }

  const runContext = extractRunContext(event);
  if (runContext) {
    accumulator.actorId = runContext.actorId || accumulator.actorId;
    accumulator.commandId = runContext.commandId || accumulator.commandId;
    accumulator.correlationId =
      runContext.correlationId || accumulator.correlationId;
  }

  const stepRequest = extractStepRequest(event);
  if (stepRequest) {
    const stepIdentity =
      stepRequest.stepId ||
      stepRequest.stepType ||
      `Step ${accumulator.steps.length + 1}`;
    const existingStep = accumulator.steps.find(
      (step) => step.id === stepIdentity || step.name === stepIdentity,
    );
    if (!existingStep) {
      accumulator.steps.push({
        id: stepRequest.stepId || stepIdentity,
        name: stepRequest.stepId || stepRequest.stepType || stepIdentity,
        startedAt: event.timestamp || Date.now(),
        status: "running",
        stepType: stepRequest.stepType || undefined,
      });
    }
  }

  const completedStep = extractStepCompleted(event);
  if (completedStep) {
    const existingStep = accumulator.steps.find(
      (step) =>
        step.id === completedStep.stepId || step.name === completedStep.stepId,
    );
    if (existingStep) {
      existingStep.error = completedStep.error;
      existingStep.finishedAt = event.timestamp || Date.now();
      existingStep.output = completedStep.output;
      existingStep.status = completedStep.success === false ? "error" : "done";
    } else {
      accumulator.steps.push({
        error: completedStep.error,
        finishedAt: event.timestamp || Date.now(),
        id: completedStep.stepId,
        name: completedStep.stepId,
        output: completedStep.output,
        startedAt: event.timestamp || Date.now(),
        status: completedStep.success === false ? "error" : "done",
      });
    }

    setFinalOutput(accumulator, completedStep.output, "step_completed");
  }

  const stepOutput = extractStepCompletedOutput(event);
  if (stepOutput && !accumulator.assistantText) {
    accumulator.assistantText = stepOutput;
  }

  const reasoningDelta = extractReasoningDelta(event);
  if (reasoningDelta) {
    accumulator.thinking += reasoningDelta;
  }

  const toolApprovalRequest = extractToolApprovalRequest(event);
  if (toolApprovalRequest) {
    accumulator.pendingApproval = toolApprovalRequest;
  }

  const runIntervention = extractRunInterventionRequest(
    event,
    accumulator.actorId,
  );
  if (runIntervention) {
    accumulator.pendingRunIntervention = runIntervention;
  }

  if (event.type === AGUIEventType.CUSTOM) {
    const custom = parseCustomEvent(event);
    if (custom.name === "studio.human.resume") {
      accumulator.pendingRunIntervention = undefined;
    }
  }

  return accumulator;
}
