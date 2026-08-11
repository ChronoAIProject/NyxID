import {
  assertTaskPlanRelationships,
  decodeTaskPlan,
  decodeTaskStep,
} from "@/lib/assistant/task-plan";
import type { TaskPlan, TaskStep } from "@/types/assistant";

type JsonRecord = Record<string, unknown>;

export interface AssistantTaskProjection {
  readonly actorId: string | null;
  readonly stateVersion: number;
  readonly progressSequence: number;
  readonly activeTurn: JsonRecord | null;
  readonly latestTurn: JsonRecord | null;
  readonly task: TaskPlan | null;
  readonly steps: ReadonlyMap<string, TaskStep>;
  readonly pendingInput: JsonRecord | null;
  readonly pendingApproval: JsonRecord | null;
  readonly latestInputResolution: JsonRecord | null;
  readonly latestApprovalResolution: JsonRecord | null;
  readonly pendingActions: readonly JsonRecord[];
}

export class TaskStateProtocolError extends Error {
  readonly code: string;

  constructor(message: string, code = "NYXID_STATE_SNAPSHOT_INVALID") {
    super(message);
    this.name = "TaskStateProtocolError";
    this.code = code;
  }
}

export function createTaskProjection(
  actorId: string | null = null,
): AssistantTaskProjection {
  return {
    actorId,
    stateVersion: 0,
    progressSequence: 0,
    activeTurn: null,
    latestTurn: null,
    task: null,
    steps: new Map(),
    pendingInput: null,
    pendingApproval: null,
    latestInputResolution: null,
    latestApprovalResolution: null,
    pendingActions: [],
  };
}

export function reduceTaskFrame(
  projection: AssistantTaskProjection,
  name: string,
  payload: unknown,
  rawSequence: unknown,
): AssistantTaskProjection {
  if (name !== "nyxid.task.snapshot" && name !== "nyxid.task.step.changed") {
    return projection;
  }
  const sequence = version(rawSequence);
  if (sequence === null) {
    throw new TaskStateProtocolError(
      "Actor progress sequence is invalid.",
      "NYXID_SEQUENCE_INVALID",
    );
  }
  if (sequence <= projection.progressSequence) return projection;

  if (name === "nyxid.task.snapshot") {
    const task = decodeTaskPlan(payload);
    if (projection.actorId && projection.actorId !== task.actorId) {
      throw new TaskStateProtocolError(
        "TaskPlan belongs to a different conversation.",
        "NYXID_STATE_IDENTITY_CONFLICT",
      );
    }
    return {
      ...projection,
      actorId: task.actorId,
      progressSequence: sequence,
      task,
      steps: new Map(task.steps.map((step) => [step.stepId, step])),
    };
  }

  const change = record(payload, "task step change");
  const changeKind = change.changeKind;
  if (
    changeKind !== "status" &&
    changeKind !== "substep" &&
    changeKind !== "added" &&
    changeKind !== "cancelled"
  ) {
    throw new TaskStateProtocolError("Task step change kind is invalid.");
  }
  if (!projection.task) {
    throw new TaskStateProtocolError(
      "Task step change arrived before its TaskPlan snapshot.",
    );
  }
  if (
    change.taskId !== projection.task.taskId ||
    version(change.planRevision) !== projection.task.planRevision
  ) {
    throw new TaskStateProtocolError(
      "Task step change does not match the active TaskPlan.",
    );
  }
  const step = decodeTaskStep(change.step);
  const steps = new Map(projection.steps);
  steps.set(step.stepId, step);
  const ordered = [...steps.values()].sort(
    (left, right) =>
      left.order - right.order || left.stepId.localeCompare(right.stepId),
  );
  const task = { ...projection.task, steps: ordered };
  assertTaskPlanRelationships(task);
  return {
    ...projection,
    progressSequence: sequence,
    task,
    steps,
  };
}

export function applyCurrentTaskState(
  projection: AssistantTaskProjection,
  input: unknown,
): { readonly projection: AssistantTaskProjection; readonly reload: boolean } {
  const envelope = record(input, "current-state envelope");
  if (envelope.status === "reload_required") {
    return { projection, reload: true };
  }
  if (envelope.status === "not_found") {
    return {
      projection: createTaskProjection(projection.actorId),
      reload: false,
    };
  }
  if (envelope.status === "not_modified") {
    const stateVersion = version(envelope.stateVersion);
    if (stateVersion !== projection.stateVersion) {
      throw new TaskStateProtocolError(
        "Current-state version does not match the local projection.",
        "NYXID_STATE_VERSION_CONFLICT",
      );
    }
    return { projection, reload: false };
  }
  if (envelope.status !== "current") {
    throw new TaskStateProtocolError(
      "Current-state status is outside the supported contract.",
      "NYXID_STATE_STATUS_INVALID",
    );
  }

  const snapshot = record(envelope.snapshot, "current-state snapshot");
  const stateVersion = version(envelope.stateVersion);
  const snapshotVersion = version(snapshot.stateVersion);
  const progressSequence = version(snapshot.progressSequence);
  if (
    stateVersion === null ||
    snapshotVersion !== stateVersion ||
    progressSequence === null
  ) {
    throw new TaskStateProtocolError(
      "Current-state snapshot versions are invalid.",
    );
  }
  if (
    stateVersion < projection.stateVersion ||
    progressSequence < projection.progressSequence
  ) {
    return { projection, reload: false };
  }
  const actorId = identity(snapshot.actorId);
  if (!actorId || (projection.actorId && projection.actorId !== actorId)) {
    throw new TaskStateProtocolError(
      "Current-state snapshot belongs to a different conversation.",
      "NYXID_STATE_IDENTITY_CONFLICT",
    );
  }

  const activeTask = optionalRecord(snapshot.activeTask);
  const task = activeTask ? decodeTaskPlan(activeTask) : null;
  if (task && task.actorId !== actorId) {
    throw new TaskStateProtocolError(
      "Current-state TaskPlan belongs to a different conversation.",
      "NYXID_STATE_IDENTITY_CONFLICT",
    );
  }
  const pendingActions = distinctActions([
    ...optionalArray(snapshot.pendingActions),
    ...optionalArray(snapshot.recentActions),
  ]);
  return {
    projection: {
      actorId,
      stateVersion,
      progressSequence,
      activeTurn: cloneNullableRecord(snapshot.activeTurn),
      latestTurn: cloneNullableRecord(snapshot.latestTurn),
      task,
      steps: new Map(task?.steps.map((step) => [step.stepId, step]) ?? []),
      pendingInput: cloneNullableRecord(snapshot.pendingInput),
      pendingApproval: cloneNullableRecord(snapshot.pendingApproval),
      latestInputResolution: cloneNullableRecord(
        snapshot.latestInputResolution,
      ),
      latestApprovalResolution: cloneNullableRecord(
        snapshot.latestApprovalResolution,
      ),
      pendingActions,
    },
    reload: false,
  };
}

export function taskCan(
  projection: AssistantTaskProjection | null | undefined,
  action: "retry" | "skip" | "stop",
  stepId?: string,
): boolean {
  if (!projection) return false;
  if (action === "stop" && !stepId) {
    return [...projection.steps.values()].some(
      (step) => step.availableActions.stop,
    );
  }
  if (!stepId) return false;
  return projection.steps.get(stepId)?.availableActions[action] === true;
}

function distinctActions(input: unknown[]): JsonRecord[] {
  const actions = new Map<string, JsonRecord>();
  for (const item of input) {
    const action = optionalRecord(item);
    const actionRequestId = identity(action?.actionRequestId);
    if (!action || !actionRequestId || actions.has(actionRequestId)) continue;
    actions.set(actionRequestId, cloneRecord(action));
  }
  return [...actions.values()];
}

function record(input: unknown, label: string): JsonRecord {
  const value = optionalRecord(input);
  if (!value) throw new TaskStateProtocolError(`${label} must be an object.`);
  return value;
}

function optionalRecord(input: unknown): JsonRecord | null {
  return input && typeof input === "object" && !Array.isArray(input)
    ? (input as JsonRecord)
    : null;
}

function optionalArray(input: unknown): unknown[] {
  return Array.isArray(input) ? input : [];
}

function version(input: unknown): number | null {
  const parsed =
    typeof input === "string" && /^\d+$/.test(input) ? Number(input) : input;
  return typeof parsed === "number" &&
    Number.isSafeInteger(parsed) &&
    parsed >= 0
    ? parsed
    : null;
}

function identity(input: unknown): string | null {
  if (typeof input !== "string" || input.length < 1 || input.length > 256) {
    return null;
  }
  return [...input].some((character) => {
    const code = character.charCodeAt(0);
    return code <= 31 || code === 127 || /[\s/\\?#]/u.test(character);
  })
    ? null
    : input;
}

function cloneRecord(value: JsonRecord): JsonRecord {
  return JSON.parse(JSON.stringify(value)) as JsonRecord;
}

function cloneNullableRecord(value: unknown): JsonRecord | null {
  const parsed = optionalRecord(value);
  return parsed ? cloneRecord(parsed) : null;
}
