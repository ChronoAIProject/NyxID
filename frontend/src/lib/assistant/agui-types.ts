export const AGUIEventType = {
  RUN_STARTED: "RUN_STARTED",
  RUN_FINISHED: "RUN_FINISHED",
  RUN_ERROR: "RUN_ERROR",
  RUN_STOPPED: "RUN_STOPPED",
  STEP_STARTED: "STEP_STARTED",
  STEP_FINISHED: "STEP_FINISHED",
  TEXT_MESSAGE_START: "TEXT_MESSAGE_START",
  TEXT_MESSAGE_CONTENT: "TEXT_MESSAGE_CONTENT",
  TEXT_MESSAGE_END: "TEXT_MESSAGE_END",
  STATE_SNAPSHOT: "STATE_SNAPSHOT",
  TOOL_CALL_START: "TOOL_CALL_START",
  TOOL_CALL_END: "TOOL_CALL_END",
  TOOL_APPROVAL_REQUEST: "TOOL_APPROVAL_REQUEST",
  HUMAN_INPUT_REQUEST: "HUMAN_INPUT_REQUEST",
  HUMAN_INPUT_RESPONSE: "HUMAN_INPUT_RESPONSE",
  MEDIA_CONTENT: "MEDIA_CONTENT",
  CUSTOM: "CUSTOM",
} as const;

export type AGUIEventType = (typeof AGUIEventType)[keyof typeof AGUIEventType];

interface EventBase {
  readonly type: AGUIEventType;
  readonly timestamp?: number;
}

export interface RunStartedEvent extends EventBase {
  readonly type: typeof AGUIEventType.RUN_STARTED;
  readonly actorId?: string;
  readonly commandId?: string;
  readonly correlationId?: string;
  readonly threadId: string;
  readonly runId: string;
}

export interface RunFinishedEvent extends EventBase {
  readonly type: typeof AGUIEventType.RUN_FINISHED;
  readonly commandId?: string;
  readonly correlationId?: string;
  readonly threadId?: string;
  readonly runId?: string;
  readonly result?: unknown;
}

export interface RunErrorEvent extends EventBase {
  readonly type: typeof AGUIEventType.RUN_ERROR;
  readonly code?: string;
  readonly commandId?: string;
  readonly correlationId?: string;
  readonly message: string;
  readonly runId?: string;
}

export interface RunStoppedEvent extends EventBase {
  readonly type: typeof AGUIEventType.RUN_STOPPED;
  readonly reason?: string;
  readonly runId?: string;
}

export interface StepStartedEvent extends EventBase {
  readonly type: typeof AGUIEventType.STEP_STARTED;
  readonly stepName: string;
}

export interface StepFinishedEvent extends EventBase {
  readonly type: typeof AGUIEventType.STEP_FINISHED;
  readonly stepName: string;
}

export interface TextMessageStartEvent extends EventBase {
  readonly type: typeof AGUIEventType.TEXT_MESSAGE_START;
  readonly messageId: string;
  readonly role: string;
}

export interface TextMessageContentEvent extends EventBase {
  readonly type: typeof AGUIEventType.TEXT_MESSAGE_CONTENT;
  readonly messageId: string;
  readonly delta: string;
}

export interface TextMessageEndEvent extends EventBase {
  readonly type: typeof AGUIEventType.TEXT_MESSAGE_END;
  readonly messageId: string;
  readonly delta?: string;
  readonly message?: string;
}

export interface StateSnapshotEvent extends EventBase {
  readonly type: typeof AGUIEventType.STATE_SNAPSHOT;
  readonly snapshot: unknown;
}

export interface ToolCallStartEvent extends EventBase {
  readonly type: typeof AGUIEventType.TOOL_CALL_START;
  readonly toolCallId: string;
  readonly toolName: string;
}

export interface ToolCallEndEvent extends EventBase {
  readonly type: typeof AGUIEventType.TOOL_CALL_END;
  readonly toolCallId: string;
  readonly result?: string;
}

export interface ToolApprovalRequestEvent extends EventBase {
  readonly type: typeof AGUIEventType.TOOL_APPROVAL_REQUEST;
  readonly argumentsJson: string;
  readonly isDestructive: boolean;
  readonly requestId: string;
  readonly timeoutSeconds: number;
  readonly toolCallId: string;
  readonly toolName: string;
}

export interface HumanInputRequestEvent extends EventBase {
  readonly type: typeof AGUIEventType.HUMAN_INPUT_REQUEST;
  readonly metadata?: Record<string, string>;
  readonly prompt: string;
  readonly runId: string;
  readonly stepId: string;
  readonly suspensionType: string;
  readonly timeoutSeconds: number;
}

export interface HumanInputResponseEvent extends EventBase {
  readonly type: typeof AGUIEventType.HUMAN_INPUT_RESPONSE;
  readonly approved: boolean;
  readonly runId: string;
  readonly stepId: string;
  readonly userInput?: string;
}

export interface MediaContentEvent extends EventBase {
  readonly type: typeof AGUIEventType.MEDIA_CONTENT;
  readonly dataBase64?: string;
  readonly kind?: string;
  readonly mediaType?: string;
  readonly name?: string;
  readonly text?: string;
  readonly uri?: string;
}

export interface CustomEvent extends EventBase {
  readonly type: typeof AGUIEventType.CUSTOM;
  readonly name: string;
  readonly value?: unknown;
}

export type AGUIEvent =
  | RunStartedEvent
  | RunFinishedEvent
  | RunErrorEvent
  | RunStoppedEvent
  | StepStartedEvent
  | StepFinishedEvent
  | TextMessageStartEvent
  | TextMessageContentEvent
  | TextMessageEndEvent
  | StateSnapshotEvent
  | ToolCallStartEvent
  | ToolCallEndEvent
  | ToolApprovalRequestEvent
  | HumanInputRequestEvent
  | HumanInputResponseEvent
  | MediaContentEvent
  | CustomEvent;

export const CustomEventName = {
  RunContext: "aevatar.run.context",
  StepRequest: "aevatar.step.request",
  StepCompleted: "aevatar.step.completed",
  HumanInputRequest: "aevatar.human_input.request",
  WaitingSignal: "aevatar.workflow.waiting_signal",
  SignalBuffered: "aevatar.workflow.signal.buffered",
  LlmReasoning: "aevatar.llm.reasoning",
} as const;

export type CustomEventName =
  (typeof CustomEventName)[keyof typeof CustomEventName];

export interface RunContextData {
  readonly actorId?: string;
  readonly workflowName?: string;
  readonly commandId?: string;
}

export interface StepRequestData {
  readonly runId?: string;
  readonly stepId?: string;
  readonly stepType?: string;
  readonly input?: string;
  readonly targetRole?: string;
}

export interface StepCompletedData {
  readonly runId?: string;
  readonly stepId?: string;
  readonly success?: boolean;
  readonly output?: string;
  readonly error?: string;
}

export interface HumanInputRequestData {
  readonly runId?: string;
  readonly stepId?: string;
  readonly suspensionType?: string;
  readonly prompt?: string;
  readonly timeoutSeconds?: number;
  readonly metadata?: Record<string, string>;
}

export interface WaitingSignalData {
  readonly runId?: string;
  readonly stepId?: string;
  readonly signalName?: string;
  readonly prompt?: string;
  readonly timeoutMs?: number;
}

export interface LlmReasoningData {
  readonly role?: string;
  readonly delta?: string;
}

export interface SignalBufferedData {
  readonly runId?: string;
  readonly stepId?: string;
  readonly signalName?: string;
  readonly payload?: string;
  readonly receivedAtUnixTimeMs?: number;
}

type JsonRecord = Record<string, unknown>;

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : {};
}

const CUSTOM_EVENT_MAPPERS: Readonly<
  Record<string, (value: unknown) => unknown>
> = {
  [CustomEventName.RunContext]: (value) => {
    const data = asRecord(value);
    return {
      actorId: data["actorId"],
      workflowName: data["workflowName"],
      commandId: data["commandId"],
    };
  },
  [CustomEventName.StepRequest]: (value) => {
    const data = asRecord(value);
    return {
      runId: data["runId"],
      stepId: data["stepId"],
      stepType: data["stepType"],
      input: data["input"],
      targetRole: data["targetRole"],
    };
  },
  [CustomEventName.StepCompleted]: (value) => {
    const data = asRecord(value);
    return {
      runId: data["runId"],
      stepId: data["stepId"],
      success: data["success"],
      output: data["output"],
      error: data["error"],
    };
  },
  [CustomEventName.HumanInputRequest]: (value) => {
    const data = asRecord(value);
    return {
      runId: data["runId"],
      stepId: data["stepId"],
      suspensionType: data["suspensionType"],
      prompt: data["prompt"],
      timeoutSeconds: data["timeoutSeconds"],
      metadata: data["metadata"],
    };
  },
  [CustomEventName.WaitingSignal]: (value) => {
    const data = asRecord(value);
    return {
      runId: data["runId"],
      stepId: data["stepId"],
      signalName: data["signalName"],
      prompt: data["prompt"],
      timeoutMs: data["timeoutMs"],
    };
  },
  [CustomEventName.LlmReasoning]: (value) => {
    const data = asRecord(value);
    return { role: data["role"], delta: data["delta"] };
  },
  [CustomEventName.SignalBuffered]: (value) => {
    const data = asRecord(value);
    return {
      runId: data["runId"],
      stepId: data["stepId"],
      signalName: data["signalName"],
      payload: data["payload"],
      receivedAtUnixTimeMs: data["receivedAtUnixTimeMs"],
    };
  },
};

export function parseCustomEvent(event: CustomEvent): {
  readonly name: string;
  readonly data: unknown;
} {
  const mapper = CUSTOM_EVENT_MAPPERS[event.name];
  return {
    name: event.name,
    data: mapper ? mapper(event.value) : event.value,
  };
}
