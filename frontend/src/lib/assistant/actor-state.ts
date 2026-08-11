import { assistantActionRequestSchema } from "@/schemas/assistant-actions";

export type ActorRecord = Record<string, unknown>;

export interface ActorConflict {
  readonly code: string;
  readonly [key: string]: unknown;
}

export interface ActorProjection {
  readonly actorId: string | null;
  readonly scopeId: string | null;
  readonly stateVersion: number;
  readonly progressSequence: number;
  readonly sequenceFingerprints: ReadonlySet<string>;
  readonly activeTurn: ActorRecord | null;
  readonly latestTurn: ActorRecord | null;
  readonly recentTerminalTurns: readonly ActorRecord[];
  readonly task: ActorRecord | null;
  readonly steps: ReadonlyMap<string, ActorRecord>;
  readonly pendingInput: ActorRecord | null;
  readonly pendingApproval: ActorRecord | null;
  readonly latestInputResolution: ActorRecord | null;
  readonly latestApprovalResolution: ActorRecord | null;
  readonly taskStatus: string | null;
  readonly attentionKind: string;
  readonly attentionSince: string | null;
  readonly activeStepSummary: string | null;
  readonly actions: ReadonlyMap<string, ActorRecord>;
  readonly recentActions: readonly ActorRecord[];
  readonly controlFence: ActorRecord | null;
  readonly latestControlResult: ActorRecord | null;
  readonly continuation: ActorRecord | null;
  readonly latestStepControlResult: ActorRecord | null;
  readonly recentStepControlResults: readonly ActorRecord[];
  readonly conflicts: readonly ActorConflict[];
}

export interface ActorEvent {
  readonly type:
    | "task_snapshot"
    | "task_step_changed"
    | "control_changed"
    | "continuation_changed"
    | "step_control_changed"
    | "input_requested"
    | "input_changed"
    | "approval_requested"
    | "approval_changed"
    | "action_request";
  readonly sequence: number;
  readonly payload: ActorRecord;
}

export interface CurrentStateApplication {
  readonly projection: ActorProjection;
  readonly reloadWithoutCursor: boolean;
  readonly status: "current" | "not_modified" | "reload_required" | "not_found" | "invalid";
}

const ACTOR_EVENT_TYPES = new Map<string, ActorEvent["type"]>([
  ["nyxid.task.snapshot", "task_snapshot"],
  ["nyxid.task.step.changed", "task_step_changed"],
  ["nyxid.control.changed", "control_changed"],
  ["nyxid.continuation.changed", "continuation_changed"],
  ["nyxid.step.control.changed", "step_control_changed"],
  ["nyxid.input.request", "input_requested"],
  ["nyxid.input.changed", "input_changed"],
  ["nyxid.approval.request", "approval_requested"],
  ["nyxid.approval.changed", "approval_changed"],
  ["nyxid.action.request", "action_request"],
]);

const MAX_CONFLICTS = 32;
const ACTION_IDENTITY_KEYS = [
  "actorId",
  "originTurnId",
  "taskId",
  "stepId",
  "actionRequestId",
] as const;

export function createActorProjection(actorId: string | null = null): ActorProjection {
  return {
    actorId,
    scopeId: null,
    stateVersion: 0,
    progressSequence: 0,
    sequenceFingerprints: new Set(),
    activeTurn: null,
    latestTurn: null,
    recentTerminalTurns: [],
    task: null,
    steps: new Map(),
    pendingInput: null,
    pendingApproval: null,
    latestInputResolution: null,
    latestApprovalResolution: null,
    taskStatus: null,
    attentionKind: "none",
    attentionSince: null,
    activeStepSummary: null,
    actions: new Map(),
    recentActions: [],
    controlFence: null,
    latestControlResult: null,
    continuation: null,
    latestStepControlResult: null,
    recentStepControlResults: [],
    conflicts: [],
  };
}

export function decodeActorEvent(
  name: string,
  payload: unknown,
  rawSequence: unknown,
): ActorEvent | null {
  const type = ACTOR_EVENT_TYPES.get(name);
  if (!type) return null;
  const sequence = Number(rawSequence);
  if (!Number.isSafeInteger(sequence) || sequence < 0) {
    throw new Error("NYXID_SEQUENCE_INVALID");
  }
  const record = cloneRecord(payload);
  validateEventPayload(type, record);
  return { type, sequence, payload: record };
}

export function reduceActorEvent(
  projection: ActorProjection,
  event: ActorEvent,
): ActorProjection {
  if (event.sequence < projection.progressSequence) return projection;
  const fingerprint = eventFingerprint(event);
  if (
    event.sequence === projection.progressSequence &&
    projection.sequenceFingerprints.has(fingerprint)
  ) {
    return projection;
  }

  const next = cloneProjection(projection, event.sequence, fingerprint);
  switch (event.type) {
    case "task_snapshot":
      return applyTaskSnapshot(next, event.payload);
    case "task_step_changed":
      return applyStepChanged(next, event.payload);
    case "control_changed":
      return { ...next, latestControlResult: event.payload };
    case "continuation_changed":
      return { ...next, continuation: event.payload };
    case "step_control_changed":
      return {
        ...next,
        latestStepControlResult: event.payload,
        recentStepControlResults: [
          ...next.recentStepControlResults,
          event.payload,
        ].slice(-32),
      };
    case "input_requested":
      return {
        ...next,
        pendingInput: event.payload,
        attentionKind: "input",
        attentionSince: stringValue(event.payload["askedAt"]),
        activeStepSummary: stringValue(event.payload["prompt"]),
      };
    case "input_changed":
      return updateAttention({
        ...next,
        latestInputResolution: event.payload,
        pendingInput:
          next.pendingInput?.["requestId"] === event.payload["requestId"]
            ? null
            : next.pendingInput,
      });
    case "approval_requested":
      return {
        ...next,
        pendingApproval: pendingApprovalFromEvent(event.payload),
        attentionKind: "approval",
        attentionSince: stringValue(event.payload["askedAt"]),
        activeStepSummary:
          stringValue(asRecord(event.payload["presentation"])?.["action"]) ??
          stringValue(event.payload["toolName"]),
      };
    case "approval_changed":
      return updateAttention({
        ...next,
        latestApprovalResolution: event.payload,
        pendingApproval:
          approvalRequestId(next.pendingApproval) === event.payload["requestId"]
            ? null
            : next.pendingApproval,
      });
    case "action_request": {
      const parsed = assistantActionRequestSchema.safeParse(event.payload);
      if (!parsed.success) {
        return withConflict(next, "NYXID_ACTION_REQUEST_INVALID", {
          sequence: event.sequence,
        });
      }
      const request = parsed.data as ActorRecord;
      const requestId = parsed.data.actionRequestId;
      const actorId = identity(parsed.data.actorId);
      if (next.actorId && actorId && next.actorId !== actorId) {
        return withConflict(next, "NYXID_ACTOR_ID_CONFLICT", {
          actionRequestId: requestId,
          sequence: event.sequence,
        });
      }
      const actions = new Map(next.actions);
      const existing = actions.get(requestId);
      if (!existing) {
        actions.set(requestId, actionRecordFromRequest(request));
        return { ...next, actorId: next.actorId ?? actorId, actions };
      }
      const existingRequest = asRecord(existing["request"]);
      if (
        (existingRequest && stableJson(existingRequest) === stableJson(request)) ||
        (!existingRequest && summaryMatchesRequest(existing, request))
      ) {
        actions.set(requestId, {
          ...actionRecordFromRequest(request),
          reports: cloneRecordArray(existing["reports"]),
          postconditionResult: cloneNullableRecord(
            existing["postconditionResult"],
          ),
        });
        return { ...next, actorId: next.actorId ?? actorId, actions };
      }
      actions.set(requestId, {
        ...existing,
        conflicted: true,
        executable: false,
        errorCode: "NYXID_ACTION_ID_CONFLICT",
      });
      return withConflict({ ...next, actions }, "NYXID_ACTION_ID_CONFLICT", {
        actionRequestId: requestId,
        sequence: event.sequence,
      });
    }
  }
}

export function applyCurrentStateResult(
  projection: ActorProjection,
  input: unknown,
): CurrentStateApplication {
  const envelope = asRecord(input);
  if (!envelope) {
    return invalidState(projection, "NYXID_STATE_ENVELOPE_INVALID");
  }
  const status = envelope?.["status"];
  if (status === "not_modified") {
    const version = safeVersion(envelope["stateVersion"]);
    if (version === null || version !== projection.stateVersion) {
      return invalidState(projection, "NYXID_STATE_VERSION_CONFLICT");
    }
    return { projection, reloadWithoutCursor: false, status };
  }
  if (status === "reload_required") {
    return { projection, reloadWithoutCursor: true, status };
  }
  if (status === "not_found") {
    return {
      projection: createActorProjection(projection.actorId),
      reloadWithoutCursor: false,
      status,
    };
  }
  if (status !== "current") {
    return invalidState(projection, "NYXID_STATE_STATUS_INVALID");
  }

  const snapshot = asRecord(envelope["snapshot"]);
  const envelopeVersion = safeVersion(envelope["stateVersion"]);
  const snapshotVersion = safeVersion(snapshot?.["stateVersion"]);
  const progressSequence = safeVersion(snapshot?.["progressSequence"]);
  if (
    !snapshot ||
    envelopeVersion === null ||
    snapshotVersion === null ||
    progressSequence === null
  ) {
    return invalidState(projection, "NYXID_STATE_VERSION_INVALID");
  }
  if (envelopeVersion !== snapshotVersion) {
    return invalidState(projection, "NYXID_STATE_VERSION_CONFLICT");
  }
  if (
    projection.stateVersion > snapshotVersion ||
    projection.progressSequence > progressSequence
  ) {
    return { projection, reloadWithoutCursor: false, status };
  }

  const actorId = identity(snapshot["actorId"]);
  const scopeId = identity(snapshot["scopeId"]);
  if (!actorId || (projection.actorId && projection.actorId !== actorId)) {
    return invalidState(projection, "NYXID_ACTOR_ID_CONFLICT");
  }
  if (!scopeId || (projection.scopeId && projection.scopeId !== scopeId)) {
    return invalidState(projection, "NYXID_SCOPE_ID_CONFLICT");
  }

  let next: ActorProjection = {
    ...createActorProjection(actorId),
    scopeId,
    stateVersion: snapshotVersion,
    progressSequence,
    activeTurn: cloneNullableRecord(snapshot["activeTurn"]),
    latestTurn: cloneNullableRecord(snapshot["latestTurn"]),
    recentTerminalTurns: cloneRecordArray(snapshot["recentTerminalTurns"]),
    pendingInput: cloneNullableRecord(snapshot["pendingInput"]),
    pendingApproval: cloneNullableRecord(snapshot["pendingApproval"]),
    latestInputResolution: cloneNullableRecord(snapshot["latestInputResolution"]),
    latestApprovalResolution: cloneNullableRecord(
      snapshot["latestApprovalResolution"],
    ),
    taskStatus:
      stringValue(snapshot["taskStatus"]) ??
      stringValue(asRecord(snapshot["activeTask"])?.["status"]),
    attentionKind: stringValue(snapshot["attentionKind"]) ?? "none",
    attentionSince: stringValue(snapshot["attentionSince"]),
    activeStepSummary: stringValue(snapshot["activeStepSummary"]),
    recentActions: cloneRecordArray(snapshot["recentActions"]),
    controlFence: cloneNullableRecord(snapshot["controlFence"]),
    latestControlResult: cloneNullableRecord(snapshot["latestControlResult"]),
    continuation: cloneNullableRecord(snapshot["continuationAdmission"]),
    latestStepControlResult: cloneNullableRecord(
      snapshot["latestStepControlResult"],
    ),
    recentStepControlResults: cloneRecordArray(
      snapshot["recentStepControlResults"],
    ),
    conflicts: projection.conflicts,
  };
  const activeTask = cloneNullableRecord(snapshot["activeTask"]);
  if (activeTask) next = applyTaskSnapshot(next, activeTask);
  next = applyActionSummaries(
    next,
    snapshot["pendingActions"],
    projection.actions,
  );
  return { projection: next, reloadWithoutCursor: false, status };
}

export function actorTurnId(projection: ActorProjection): string | null {
  return (
    identity(projection.activeTurn?.["turnId"]) ??
    identity(projection.latestTurn?.["turnId"]) ??
    identity(projection.task?.["turnId"])
  );
}

export function actorCan(
  projection: ActorProjection,
  action: "stop" | "retry" | "skip",
  stepId?: string,
): boolean {
  if (action === "stop") {
    if (stepId) {
      return asRecord(projection.steps.get(stepId)?.["availableActions"])?.[
        "stop"
      ] === true;
    }
    return [...projection.steps.values()].some(
      (step) => asRecord(step["availableActions"])?.["stop"] === true,
    );
  }
  if (!stepId) return false;
  return (
    asRecord(projection.steps.get(stepId)?.["availableActions"])?.[action] ===
    true
  );
}

export function actorOperationGeneration(step: ActorRecord): number | null {
  const operation = asRecord(step["operation"]);
  const key = asRecord(operation?.["key"]);
  for (const candidate of [
    key?.["operationGeneration"],
    operation?.["generation"],
    step["operationGeneration"],
  ]) {
    if (candidate === undefined || candidate === null) continue;
    return Number.isSafeInteger(candidate) && Number(candidate) >= 1
      ? Number(candidate)
      : null;
  }
  return null;
}

function applyTaskSnapshot(
  projection: ActorProjection,
  task: ActorRecord,
): ActorProjection {
  const actorId = identity(task["actorId"]);
  const taskId = identity(task["taskId"]);
  const planRevision = positiveSafeInteger(task["planRevision"]);
  if (!actorId || !taskId || planRevision === null) {
    return withConflict(projection, "NYXID_TASK_IDENTITY_INVALID");
  }
  if (projection.actorId && projection.actorId !== actorId) {
    return withConflict(projection, "NYXID_ACTOR_ID_CONFLICT", { taskId });
  }
  if (
    projection.task?.["taskId"] === taskId &&
    positiveSafeInteger(projection.task["planRevision"]) !== null &&
    planRevision < Number(projection.task["planRevision"])
  ) {
    return withConflict(projection, "NYXID_PLAN_REVISION_CONFLICT", { taskId });
  }
  const steps = new Map<string, ActorRecord>();
  for (const step of orderedSteps(task["steps"])) {
    const record = asRecord(step);
    const stepId = identity(record?.["stepId"]);
    if (record && stepId) steps.set(stepId, cloneRecord(record));
  }
  return {
    ...projection,
    actorId: projection.actorId ?? actorId,
    task: { ...cloneRecord(task), steps: [...steps.values()] },
    steps,
    taskStatus: stringValue(task["status"]) ?? projection.taskStatus,
  };
}

function applyStepChanged(
  projection: ActorProjection,
  payload: ActorRecord,
): ActorProjection {
  const taskId = identity(payload["taskId"]);
  const step = asRecord(payload["step"]);
  const stepId = identity(step?.["stepId"]);
  if (!taskId || taskId !== projection.task?.["taskId"] || !step || !stepId) {
    return withConflict(projection, "NYXID_TASK_ID_CONFLICT");
  }
  const planRevision = positiveSafeInteger(payload["planRevision"]);
  const currentRevision = positiveSafeInteger(projection.task["planRevision"]);
  if (
    planRevision === null ||
    (currentRevision !== null && planRevision < currentRevision)
  ) {
    return withConflict(projection, "NYXID_PLAN_REVISION_CONFLICT", { taskId });
  }
  const steps = new Map(projection.steps);
  steps.set(stepId, { ...(steps.get(stepId) ?? {}), ...cloneRecord(step) });
  const taskSteps = orderedSteps([...steps.values()]);
  return {
    ...projection,
    steps: new Map(
      taskSteps.map((orderedStep) => [String(orderedStep["stepId"]), orderedStep]),
    ),
    task: projection.task
      ? { ...projection.task, planRevision, steps: taskSteps }
      : null,
  };
}

function applyActionSummaries(
  projection: ActorProjection,
  input: unknown,
  cachedActions: ReadonlyMap<string, ActorRecord>,
): ActorProjection {
  const actions = new Map<string, ActorRecord>();
  if (Array.isArray(input)) {
    for (const value of input) {
      const summary = asRecord(value);
      const requestId = identity(summary?.["actionRequestId"]);
      if (
        !summary ||
        !requestId ||
        !identity(summary["originTurnId"]) ||
        !identity(summary["taskId"]) ||
        !identity(summary["stepId"])
      ) {
        return withConflict(projection, "NYXID_ACTION_SUMMARY_INVALID");
      }
      if (actions.has(requestId)) {
        const existing = actions.get(requestId) ?? {};
        actions.set(requestId, {
          ...existing,
          conflicted: true,
          executable: false,
          errorCode: "NYXID_ACTION_ID_CONFLICT",
        });
        return withConflict(
          { ...projection, actions },
          "NYXID_ACTION_ID_CONFLICT",
          { actionRequestId: requestId },
        );
      }
      const record: ActorRecord = {
        schemaVersion: summary["schemaVersion"],
        actorId: projection.actorId,
        originTurnId: summary["originTurnId"],
        taskId: summary["taskId"],
        stepId: summary["stepId"],
        actionRequestId: requestId,
        action: summary["action"],
        params: null,
        request: null,
        reports: cloneRecordArray(summary["reports"]),
        postconditionResult: cloneNullableRecord(summary["postconditionResult"]),
        executable: false,
        conflicted: false,
        errorCode: null,
      };
      const cachedRequest = asRecord(cachedActions.get(requestId)?.["request"]);
      if (cachedRequest && summaryMatchesRequest(record, cachedRequest)) {
        actions.set(requestId, {
          ...actionRecordFromRequest(cachedRequest),
          reports: record["reports"],
          postconditionResult: record["postconditionResult"],
        });
      } else {
        actions.set(requestId, record);
      }
    }
  }
  return { ...projection, actions };
}

function pendingApprovalFromEvent(input: ActorRecord): ActorRecord {
  const presentation = asRecord(input["presentation"]) ?? {};
  return {
    approvalRequestId: approvalRequestId(input),
    turnId: input["turnId"],
    taskId: input["taskId"],
    stepId: input["stepId"],
    toolName: stringValue(input["toolName"]) ?? "",
    toolCallId: stringValue(input["toolCallId"]) ?? "",
    askedAt: stringValue(input["askedAt"]),
    expiresAt: stringValue(input["expiresAt"]),
    action: stringValue(presentation["action"]) ?? "",
    target: stringValue(presentation["target"]) ?? "",
    actorLabel: stringValue(presentation["actorLabel"]) ?? "",
    reversibility: stringValue(presentation["reversibility"]) ?? "unknown",
    grantBoundary: stringValue(presentation["grantBoundary"]) ?? "",
    nyxidRequestId: stringValue(presentation["nyxidRequestId"]) ?? "",
  };
}

function actionRecordFromRequest(request: ActorRecord): ActorRecord {
  return {
    schemaVersion: request["schemaVersion"],
    actorId: request["actorId"],
    originTurnId: request["originTurnId"],
    taskId: request["taskId"],
    stepId: request["stepId"],
    actionRequestId: request["actionRequestId"],
    action: request["action"],
    params: request["params"],
    request,
    reports: [],
    postconditionResult: null,
    executable: true,
    conflicted: false,
    errorCode: null,
  };
}

function summaryMatchesRequest(
  summary: ActorRecord,
  request: ActorRecord,
): boolean {
  return (
    summary["schemaVersion"] === request["schemaVersion"] &&
    summary["action"] === request["action"] &&
    ACTION_IDENTITY_KEYS.every((key) => summary[key] === request[key])
  );
}

function validateEventPayload(type: ActorEvent["type"], payload: ActorRecord): void {
  if (type === "task_snapshot") {
    if (!identity(payload["actorId"]) || !identity(payload["taskId"])) {
      throw new Error("NYXID_TASK_IDENTITY_INVALID");
    }
    const revision = Number(payload["planRevision"]);
    if (!Number.isSafeInteger(revision) || revision < 1) {
      throw new Error("NYXID_PLAN_REVISION_INVALID");
    }
  }
  if (type === "task_step_changed") {
    if (!identity(payload["taskId"]) || !identity(asRecord(payload["step"])?.["stepId"])) {
      throw new Error("NYXID_STEP_IDENTITY_INVALID");
    }
  }
  if (type === "input_requested" && !identity(payload["requestId"])) {
    throw new Error("NYXID_INPUT_IDENTITY_INVALID");
  }
  if (type === "approval_requested" && !approvalRequestId(payload)) {
    throw new Error("NYXID_APPROVAL_IDENTITY_INVALID");
  }
}

function cloneProjection(
  projection: ActorProjection,
  sequence: number,
  fingerprint: string,
): ActorProjection {
  return {
    ...projection,
    progressSequence: sequence,
    sequenceFingerprints:
      sequence > projection.progressSequence
        ? new Set([fingerprint])
        : new Set([...projection.sequenceFingerprints, fingerprint]),
    steps: new Map(projection.steps),
    actions: new Map(projection.actions),
  };
}

function updateAttention(projection: ActorProjection): ActorProjection {
  if (projection.pendingInput) {
    return { ...projection, attentionKind: "input" };
  }
  if (projection.pendingApproval) {
    return { ...projection, attentionKind: "approval" };
  }
  if (projection.actions.size > 0) {
    return { ...projection, attentionKind: "action" };
  }
  return {
    ...projection,
    attentionKind: "none",
    attentionSince: null,
    activeStepSummary: null,
  };
}

function invalidState(
  projection: ActorProjection,
  code: string,
): CurrentStateApplication {
  return {
    projection: withConflict(projection, code),
    reloadWithoutCursor: false,
    status: "invalid",
  };
}

function withConflict(
  projection: ActorProjection,
  code: string,
  details: ActorRecord = {},
): ActorProjection {
  const conflict = { code, ...details };
  if (projection.conflicts.some((item) => stableJson(item) === stableJson(conflict))) {
    return projection;
  }
  return {
    ...projection,
    conflicts: [...projection.conflicts, conflict].slice(-MAX_CONFLICTS),
  };
}

function eventFingerprint(event: ActorEvent): string {
  return stableJson({
    type: event.type,
    sequence: event.sequence,
    payload: event.payload,
  });
}

function approvalRequestId(record: ActorRecord | null | undefined): string | null {
  return (
    identity(record?.["approvalRequestId"]) ?? identity(record?.["requestId"])
  );
}

function cloneRecord(input: unknown): ActorRecord {
  const record = asRecord(input);
  if (!record) throw new Error("NYXID_ACTOR_PAYLOAD_INVALID");
  return JSON.parse(JSON.stringify(record)) as ActorRecord;
}

function cloneNullableRecord(input: unknown): ActorRecord | null {
  return input === null || input === undefined ? null : cloneRecord(input);
}

function cloneRecordArray(input: unknown): readonly ActorRecord[] {
  return Array.isArray(input)
    ? input.flatMap((value) => {
        const record = asRecord(value);
        return record ? [cloneRecord(record)] : [];
      })
    : [];
}

function asRecord(input: unknown): ActorRecord | null {
  return input !== null && typeof input === "object" && !Array.isArray(input)
    ? (input as ActorRecord)
    : null;
}

function safeVersion(input: unknown): number | null {
  const value = Number(input);
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function identity(input: unknown): string | null {
  const hasForbiddenCharacter =
    typeof input === "string" &&
    [...input].some((character) => {
      const code = character.charCodeAt(0);
      return (
        /\s/u.test(character) ||
        code <= 0x1f ||
        code === 0x7f ||
        "/\\?#".includes(character)
      );
    });
  if (
    typeof input !== "string" ||
    input.length < 1 ||
    input.length > 256 ||
    hasForbiddenCharacter
  ) {
    return null;
  }
  return input;
}

function stringValue(input: unknown): string | null {
  return typeof input === "string" && input.length > 0 ? input : null;
}

function numberValue(input: unknown): number {
  const value = Number(input);
  return Number.isFinite(value) ? value : Number.MAX_SAFE_INTEGER;
}

function positiveSafeInteger(input: unknown): number | null {
  const value = Number(input);
  return Number.isSafeInteger(value) && value >= 1 ? value : null;
}

function orderedSteps(input: unknown): ActorRecord[] {
  if (!Array.isArray(input)) return [];
  return input.flatMap((value) => {
    const record = asRecord(value);
    return record ? [cloneRecord(record)] : [];
  }).sort((left, right) => {
    const order = numberValue(left["order"]) - numberValue(right["order"]);
    return order || String(left["stepId"] ?? "").localeCompare(String(right["stepId"] ?? ""));
  });
}

function stableJson(input: unknown): string {
  return JSON.stringify(sortJson(input));
}

function sortJson(input: unknown): unknown {
  if (Array.isArray(input)) return input.map(sortJson);
  const record = asRecord(input);
  if (!record) return input;
  return Object.fromEntries(
    Object.keys(record)
      .sort()
      .map((key) => [key, sortJson(record[key])]),
  );
}
