import {
  type ChatActorStep,
  type ChatTaskPlan,
  decodeChatTaskPlan,
  decodeChatTaskStep,
} from "./chat-task-plan";
import {
  ACTION_IDENTITY_KEYS,
  ChatActorProtocolError,
  type ChatActionValidationResult,
  validateActionRequest,
} from "./chat-action-validation";
import type { AssistantActionRequest } from "@/schemas/assistant-actions";

type JsonRecord = Record<string, unknown>;

const ACTOR_EVENT_NAMES = {
  "nyxid.task.snapshot": "task_snapshot",
  "nyxid.task.step.changed": "task_step_changed",
  "nyxid.control.changed": "control_changed",
  "nyxid.continuation.changed": "continuation_changed",
  "nyxid.step.control.changed": "step_control_changed",
  "nyxid.input.request": "input_request",
  "nyxid.input.changed": "input_changed",
  "nyxid.approval.request": "approval_request",
  "nyxid.approval.changed": "approval_changed",
  "nyxid.action.request": "action_request",
} as const;

export { ChatActorProtocolError, validateActionRequest };

export type { ChatActorStep, ChatAvailableActions } from "./chat-task-plan";

export type ChatPendingInput = JsonRecord & {
  requestId: string;
  prompt: string;
  options: readonly { optionId: string; label: string; description?: string }[];
  allowFreeText: boolean;
  multiSelect: boolean;
  numericThreshold?: {
    suggestedValue: number;
    minimumValue: number;
    maximumValue: number;
  } | null;
};

export type ChatPendingApproval = JsonRecord & {
  approvalRequestId: string;
  toolName: string;
  action?: string;
  target?: string;
  reversibility?: "reversible" | "irreversible" | "unknown";
  grantBoundary?: "within_grant" | "nyxid_step_up";
};

export type ChatActionSummary = {
  schemaVersion: number;
  actorId: string;
  originTurnId: string;
  taskId: string;
  stepId: string;
  actionRequestId: string;
  action: string;
  reports?: readonly JsonRecord[];
  postconditionResult?: JsonRecord | null;
  request?: AssistantActionRequest | null;
  supported: boolean;
  recovered: boolean;
  conflicted?: boolean;
};

export type ChatActorProjection = {
  actorId: string | null;
  scopeId: string | null;
  stateVersion: number;
  progressSequence: number;
  activeTurn: JsonRecord | null;
  latestTurn: JsonRecord | null;
  recentTerminalTurns: JsonRecord[];
  task: ChatTaskPlan | null;
  steps: Map<string, ChatActorStep>;
  pendingInput: ChatPendingInput | null;
  pendingApproval: ChatPendingApproval | null;
  actions: Map<string, ChatActionSummary>;
  controlFence: JsonRecord | null;
  latestControlResult: JsonRecord | null;
  continuation: JsonRecord | null;
  latestStepControlResult: JsonRecord | null;
  recentStepControlResults: JsonRecord[];
  latestInputResolution: JsonRecord | null;
  latestApprovalResolution: JsonRecord | null;
  conflicts: readonly { code: string }[];
};

export type ChatActorFrame =
  | { type: "ignored" }
  | {
      type:
        | "task_snapshot"
        | "task_step_changed"
        | "control_changed"
        | "continuation_changed"
        | "step_control_changed"
        | "input_request"
        | "input_changed"
        | "approval_request"
        | "approval_changed";
      sequence: number;
      payload: JsonRecord;
    }
  | {
      type: "action_request";
      sequence: number;
      validation: ChatActionValidationResult;
    };

export function createChatActorProjection(
  actorId: string | null = null,
): ChatActorProjection {
  return {
    actorId,
    scopeId: null,
    stateVersion: 0,
    progressSequence: 0,
    activeTurn: null,
    latestTurn: null,
    recentTerminalTurns: [],
    task: null,
    steps: new Map(),
    pendingInput: null,
    pendingApproval: null,
    actions: new Map(),
    controlFence: null,
    latestControlResult: null,
    continuation: null,
    latestStepControlResult: null,
    recentStepControlResults: [],
    latestInputResolution: null,
    latestApprovalResolution: null,
    conflicts: [],
  };
}

export function decodeActorFrame(raw: unknown): ChatActorFrame {
  const frame = optionalRecord(raw);
  const custom = optionalRecord(frame?.custom);
  const name = typeof custom?.name === "string" ? custom.name : "";
  const type = ACTOR_EVENT_NAMES[name as keyof typeof ACTOR_EVENT_NAMES];
  if (!type) return { type: "ignored" };
  const sequence = frame?.sequence;
  if (!validVersion(sequence)) {
    throw new ChatActorProtocolError(
      "Actor progress sequence is invalid.",
      "NYXID_SEQUENCE_INVALID",
    );
  }
  const payload = unpackAny(custom?.payload);
  if (type === "input_request") {
    const pendingInput = decodePendingInput(payload);
    if (!pendingInput) throw invalidNumericThreshold();
    return { type, sequence, payload: pendingInput };
  }
  return type === "action_request"
    ? { type, sequence, validation: validateActionRequest(payload) }
    : { type, sequence, payload };
}

export function reduceActorFrame(
  projection: ChatActorProjection,
  frame: ChatActorFrame,
): ChatActorProjection {
  if (
    frame.type === "ignored" ||
    frame.sequence <= projection.progressSequence
  ) {
    return projection;
  }
  const next = cloneProjection(projection);
  next.progressSequence = frame.sequence;
  switch (frame.type) {
    case "task_snapshot":
      applyTask(next, frame.payload);
      break;
    case "task_step_changed":
      applyStep(next, optionalRecord(frame.payload.step));
      break;
    case "control_changed":
      next.latestControlResult = cloneRecord(frame.payload);
      break;
    case "continuation_changed":
      next.continuation = cloneRecord(frame.payload);
      break;
    case "step_control_changed":
      next.latestStepControlResult = cloneRecord(frame.payload);
      next.recentStepControlResults = appendDistinctRecord(
        next.recentStepControlResults,
        frame.payload,
      );
      break;
    case "input_request":
      next.pendingInput = decodePendingInput(frame.payload);
      break;
    case "input_changed":
      next.latestInputResolution = cloneRecord(frame.payload);
      if (next.pendingInput?.requestId === frame.payload.requestId) {
        next.pendingInput = null;
      }
      break;
    case "approval_request":
      next.pendingApproval = normalizePendingApproval(frame.payload);
      break;
    case "approval_changed":
      next.latestApprovalResolution = cloneRecord(frame.payload);
      if (
        next.pendingApproval?.approvalRequestId ===
        (frame.payload.approvalRequestId ?? frame.payload.requestId)
      ) {
        next.pendingApproval = null;
      }
      break;
    case "action_request":
      applyActionRequest(next, frame.validation);
      break;
  }
  return next;
}

export function applyCurrentStateResult(
  projection: ChatActorProjection,
  input: unknown,
): { projection: ChatActorProjection; reloadWithoutCursor: boolean } {
  const envelope = optionalRecord(input);
  if (!envelope) {
    return {
      projection: withConflict(projection, "NYXID_STATE_STATUS_INVALID"),
      reloadWithoutCursor: false,
    };
  }
  const status = envelope.status;
  if (status === "reload_required") {
    return { projection, reloadWithoutCursor: true };
  }
  if (status === "not_found") {
    return {
      projection: createChatActorProjection(projection.actorId),
      reloadWithoutCursor: false,
    };
  }
  if (status === "not_modified") {
    return validVersion(envelope.stateVersion) &&
      envelope.stateVersion === projection.stateVersion
      ? { projection, reloadWithoutCursor: false }
      : {
          projection: withConflict(projection, "NYXID_STATE_VERSION_CONFLICT"),
          reloadWithoutCursor: false,
        };
  }
  if (status !== "current") {
    return {
      projection: withConflict(projection, "NYXID_STATE_STATUS_INVALID"),
      reloadWithoutCursor: false,
    };
  }

  const snapshot = optionalRecord(envelope.snapshot);
  if (
    !snapshot ||
    !validVersion(envelope.stateVersion) ||
    envelope.stateVersion !== snapshot.stateVersion ||
    !validVersion(snapshot.progressSequence)
  ) {
    return {
      projection: withConflict(projection, "NYXID_STATE_SNAPSHOT_INVALID"),
      reloadWithoutCursor: false,
    };
  }
  if (
    projection.stateVersion > envelope.stateVersion ||
    projection.progressSequence > snapshot.progressSequence
  ) {
    return { projection, reloadWithoutCursor: false };
  }
  const actorId = readIdentity(snapshot.actorId);
  const scopeId = readIdentity(snapshot.scopeId);
  if (
    !actorId ||
    !scopeId ||
    (projection.actorId && projection.actorId !== actorId) ||
    (projection.scopeId && projection.scopeId !== scopeId)
  ) {
    return {
      projection: withConflict(projection, "NYXID_STATE_IDENTITY_CONFLICT"),
      reloadWithoutCursor: false,
    };
  }
  if (
    projection.scopeId !== null &&
    projection.stateVersion === envelope.stateVersion &&
    projection.progressSequence === snapshot.progressSequence
  ) {
    return { projection, reloadWithoutCursor: false };
  }

  const next = createChatActorProjection(actorId);
  next.scopeId = scopeId;
  next.stateVersion = envelope.stateVersion;
  next.progressSequence = snapshot.progressSequence;
  next.activeTurn = cloneNullableRecord(snapshot.activeTurn);
  next.latestTurn = cloneNullableRecord(snapshot.latestTurn);
  next.recentTerminalTurns = Array.isArray(snapshot.recentTerminalTurns)
    ? snapshot.recentTerminalTurns
        .map(optionalRecord)
        .filter((value): value is JsonRecord => Boolean(value))
        .map(cloneRecord)
    : [];
  next.pendingInput = decodePendingInput(snapshot.pendingInput);
  next.pendingApproval = normalizePendingApproval(snapshot.pendingApproval);
  next.controlFence = cloneNullableRecord(snapshot.controlFence);
  next.latestControlResult = cloneNullableRecord(snapshot.latestControlResult);
  next.latestStepControlResult = cloneNullableRecord(
    snapshot.latestStepControlResult,
  );
  next.recentStepControlResults = Array.isArray(
    snapshot.recentStepControlResults,
  )
    ? snapshot.recentStepControlResults
        .map(optionalRecord)
        .filter((value): value is JsonRecord => Boolean(value))
        .map(cloneRecord)
        .slice(-32)
    : [];
  next.continuation = cloneNullableRecord(snapshot.continuationAdmission);
  next.latestInputResolution = cloneNullableRecord(
    snapshot.latestInputResolution,
  );
  next.latestApprovalResolution = cloneNullableRecord(
    snapshot.latestApprovalResolution,
  );
  next.conflicts = [...projection.conflicts];
  const activeTask = optionalRecord(snapshot.activeTask);
  if (activeTask) applyTask(next, activeTask);
  applyActionSummaries(
    next,
    [
      ...(Array.isArray(snapshot.pendingActions)
        ? snapshot.pendingActions
        : []),
      ...(Array.isArray(snapshot.recentActions) ? snapshot.recentActions : []),
    ],
    projection.actions,
  );
  return { projection: next, reloadWithoutCursor: false };
}

export function actorCan(
  projection: ChatActorProjection | null | undefined,
  action: "retry" | "skip" | "stop",
  stepId?: string,
): boolean {
  if (!projection) return false;
  if (action === "stop" && !stepId) {
    return [...projection.steps.values()].some(
      (step) => step.availableActions?.stop === true,
    );
  }
  if (!stepId) return false;
  return projection.steps.get(stepId)?.availableActions?.[action] === true;
}

export function chatActionIdentityKey(
  actorId: string,
  actionRequestId: string,
): string {
  return JSON.stringify([actorId, actionRequestId]);
}

function applyTask(projection: ChatActorProjection, task: JsonRecord): void {
  const decoded = decodeChatTaskPlan(task);
  projection.task = decoded;
  const steps = decoded.steps;
  projection.steps = new Map(steps.map((step) => [step.stepId, step]));
}

function applyStep(
  projection: ChatActorProjection,
  input: JsonRecord | null,
): void {
  if (!input) return;
  const stepId = readIdentity(input.stepId);
  if (!stepId) return;
  const step = decodeChatTaskStep(input);
  projection.steps.set(stepId, step);
  if (projection.task) {
    projection.task = {
      ...projection.task,
      steps: [...projection.steps.values()].sort(
        (left, right) =>
          left.order - right.order || left.stepId.localeCompare(right.stepId),
      ),
    };
  }
}

function applyActionRequest(
  projection: ChatActorProjection,
  validation: ChatActionValidationResult,
): void {
  const { request } = validation;
  if (
    projection.actorId &&
    request.actorId &&
    projection.actorId !== request.actorId
  ) {
    projection.conflicts = [
      ...projection.conflicts,
      { code: "NYXID_STATE_IDENTITY_CONFLICT" },
    ];
    return;
  }
  const existing = projection.actions.get(request.actionRequestId);
  if (
    existing &&
    (!actionIdentityMatches(existing, request) ||
      (existing.request &&
        JSON.stringify(existing.request) !== JSON.stringify(request)))
  ) {
    projection.actions.set(request.actionRequestId, {
      ...existing,
      conflicted: true,
    });
    return;
  }
  projection.actorId ||= request.actorId;
  projection.actions.set(request.actionRequestId, {
    schemaVersion: request.schemaVersion,
    ...Object.fromEntries(
      ACTION_IDENTITY_KEYS.map((key) => [key, request[key]]),
    ),
    action: request.action,
    reports: existing?.reports ?? [],
    postconditionResult: existing?.postconditionResult ?? null,
    request,
    supported: validation.supported,
    recovered: validation.recovered,
  } as ChatActionSummary);
}

function applyActionSummaries(
  projection: ChatActorProjection,
  input: unknown,
  observedActions: ReadonlyMap<string, ChatActionSummary> = new Map(),
): void {
  projection.actions = new Map();
  if (!Array.isArray(input)) return;
  for (const raw of input) {
    const summary = optionalRecord(raw);
    if (!summary) continue;
    const actionRequestId = readIdentity(summary.actionRequestId);
    const originTurnId = readIdentity(summary.originTurnId);
    const taskId = readIdentity(summary.taskId);
    const stepId = readIdentity(summary.stepId);
    if (!actionRequestId || !originTurnId || !taskId || !stepId) continue;
    const existing = projection.actions.get(actionRequestId);
    if (existing) {
      projection.actions.set(actionRequestId, {
        ...existing,
        conflicted: true,
        request: null,
      });
      projection.conflicts = [
        ...projection.conflicts,
        { code: "NYXID_ACTION_ID_CONFLICT" },
      ];
      continue;
    }
    const item: ChatActionSummary = {
      schemaVersion:
        typeof summary.schemaVersion === "number" ? summary.schemaVersion : 0,
      actorId: projection.actorId ?? "",
      originTurnId,
      taskId,
      stepId,
      actionRequestId,
      action: typeof summary.action === "string" ? summary.action : "",
      request: null,
      supported: false,
      recovered: false,
      reports: Array.isArray(summary.reports)
        ? (summary.reports.filter(optionalRecord) as JsonRecord[])
        : [],
      postconditionResult: cloneNullableRecord(summary.postconditionResult),
    };
    const reloadedValidation = optionalRecord(summary.request)
      ? validateActionRequest(summary.request)
      : null;
    if (reloadedValidation) {
      if (reloadedValidation.recovered) {
        item.recovered = true;
      } else if (actionIdentityMatches(item, reloadedValidation.request)) {
        item.request = reloadedValidation.request;
        item.supported = true;
      } else {
        item.conflicted = true;
        projection.conflicts = [
          ...projection.conflicts,
          { code: "NYXID_ACTION_ID_CONFLICT" },
        ];
      }
    }
    const observed = observedActions.get(actionRequestId);
    if (
      !item.request &&
      !item.recovered &&
      !item.conflicted &&
      observed?.request &&
      !observed.conflicted &&
      actionIdentityMatches(item, observed.request)
    ) {
      item.request = observed.request;
      item.supported = observed.supported;
      item.recovered = observed.recovered;
    }
    projection.actions.set(actionRequestId, item);
  }
}

function normalizePendingApproval(input: unknown): ChatPendingApproval | null {
  const value = optionalRecord(input);
  if (!value) return null;
  const presentation = optionalRecord(value.presentation);
  const normalized = { ...cloneRecord(value), ...(presentation ?? {}) };
  const approvalRequestId = readIdentity(
    value.approvalRequestId ?? value.requestId,
  );
  if (!approvalRequestId) return null;
  return {
    ...normalized,
    approvalRequestId,
    toolName:
      typeof normalized.toolName === "string" ? normalized.toolName : "",
  };
}

function decodePendingInput(input: unknown): ChatPendingInput | null {
  const value = optionalRecord(input);
  if (!value) return null;
  const numericThreshold = value.numericThreshold;
  if (numericThreshold === undefined || numericThreshold === null) {
    return cloneRecord(value) as ChatPendingInput;
  }
  const threshold = optionalRecord(numericThreshold);
  const suggestedValue = threshold?.suggestedValue;
  const minimumValue = threshold?.minimumValue;
  const maximumValue = threshold?.maximumValue;
  if (
    !validSafeInteger(suggestedValue) ||
    !validSafeInteger(minimumValue) ||
    !validSafeInteger(maximumValue) ||
    minimumValue > maximumValue ||
    suggestedValue < minimumValue ||
    suggestedValue > maximumValue
  ) {
    throw invalidNumericThreshold();
  }
  return {
    ...cloneRecord(value),
    numericThreshold: {
      suggestedValue,
      minimumValue,
      maximumValue,
    },
  } as ChatPendingInput;
}

function actionIdentityMatches(
  summary: ChatActionSummary,
  request: AssistantActionRequest,
): boolean {
  return (
    summary.schemaVersion === request.schemaVersion &&
    summary.action === request.action &&
    ACTION_IDENTITY_KEYS.every((key) => summary[key] === request[key])
  );
}

function unpackAny(input: unknown): JsonRecord {
  const value = requireRecord(input, "NYXID_ACTION_VARIANT_INVALID");
  const nested = optionalRecord(value.value);
  if (nested) return nested;
  const result = { ...value };
  delete result["@type"];
  return result;
}

function requireRecord(input: unknown, code: string): JsonRecord {
  const value = optionalRecord(input);
  if (!value) throw new ChatActorProtocolError("Invalid object.", code);
  return value;
}

function optionalRecord(input: unknown): JsonRecord | null {
  return input && typeof input === "object" && !Array.isArray(input)
    ? (input as JsonRecord)
    : null;
}

function validVersion(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function validSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value);
}

function readIdentity(value: unknown): string | null {
  if (typeof value !== "string" || value.length < 1 || value.length > 256) {
    return null;
  }
  const invalid = [...value].some((character) => {
    const code = character.charCodeAt(0);
    return code <= 31 || code === 127 || /[\s/\\?#]/u.test(character);
  });
  return invalid ? null : value;
}

function invalidNumericThreshold(): ChatActorProtocolError {
  return new ChatActorProtocolError(
    "NyxID numeric threshold is invalid.",
    "NYXID_INPUT_NUMERIC_THRESHOLD_INVALID",
  );
}

function cloneProjection(projection: ChatActorProjection): ChatActorProjection {
  return {
    ...projection,
    recentTerminalTurns: projection.recentTerminalTurns.map(cloneRecord),
    recentStepControlResults:
      projection.recentStepControlResults.map(cloneRecord),
    task: projection.task
      ? (JSON.parse(JSON.stringify(projection.task)) as ChatTaskPlan)
      : null,
    steps: new Map(
      [...projection.steps].map(([key, value]) => [key, cloneStep(value)]),
    ),
    pendingInput: projection.pendingInput
      ? ({ ...projection.pendingInput } as ChatPendingInput)
      : null,
    pendingApproval: projection.pendingApproval
      ? ({ ...projection.pendingApproval } as ChatPendingApproval)
      : null,
    actions: new Map(
      [...projection.actions].map(([key, value]) => [key, { ...value }]),
    ),
    conflicts: [...projection.conflicts],
  };
}

function cloneStep(step: ChatActorStep): ChatActorStep {
  return JSON.parse(JSON.stringify(step)) as ChatActorStep;
}

function cloneRecord(value: JsonRecord): JsonRecord {
  return JSON.parse(JSON.stringify(value)) as JsonRecord;
}

function cloneNullableRecord(value: unknown): JsonRecord | null {
  const record = optionalRecord(value);
  return record ? cloneRecord(record) : null;
}

function appendDistinctRecord(
  records: readonly JsonRecord[],
  value: JsonRecord,
): JsonRecord[] {
  const serialized = JSON.stringify(value);
  const next = records.some((record) => JSON.stringify(record) === serialized)
    ? records.map(cloneRecord)
    : [...records.map(cloneRecord), cloneRecord(value)];
  return next.slice(-32);
}

function withConflict(
  projection: ChatActorProjection,
  code: string,
): ChatActorProjection {
  const next = cloneProjection(projection);
  if (!next.conflicts.some((conflict) => conflict.code === code)) {
    next.conflicts = [...next.conflicts, { code }];
  }
  return next;
}
