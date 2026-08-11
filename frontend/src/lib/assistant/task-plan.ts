import type {
  TaskApprovalObservation,
  TaskAvailableActions,
  TaskNumericCondition,
  TaskOperation,
  TaskPlan,
  TaskPlanGate,
  TaskStep,
  TaskStepKind,
  TaskStepSource,
  TaskSubstep,
} from "@/types/assistant";

export class TaskPlanProtocolError extends Error {
  readonly code = "NYXID_TASK_PLAN_INVALID";

  constructor(message: string) {
    super(message);
    this.name = "TaskPlanProtocolError";
  }
}

const TASK_STATUSES = [
  "active",
  "succeeded",
  "failed",
  "stopped",
  "blocked",
] as const;
const STEP_STATUSES = [
  "planned",
  "waiting",
  "running",
  "done",
  "failed",
  "skipped",
  "cancelled",
  "uncertain",
] as const;
const STEP_KINDS = [
  "llm",
  "tool",
  "browser_action",
  "postcondition",
  "input",
  "approval",
  "web",
  "condition",
] as const;
const EFFECTS = [
  "not_started",
  "not_applied",
  "confirmed",
  "may_have_changed",
] as const;
const ADDED_BY = [
  "initial",
  "scope_resolution",
  "replan",
  "steering",
  "user_revision",
] as const;
const REVISION_CAUSES = [
  "initial",
  "scope_resolution",
  "failure_recovery",
  "steering",
  "user_revision",
] as const;

/**
 * Decode the actor-owned TaskPlan contract. Object members are additive, while
 * identities, required fields, and closed enums remain fail-closed.
 */
export function decodeTaskPlan(input: unknown): TaskPlan {
  const value = record(input, "task plan");
  const taskId = identity(value.taskId, "taskId");
  const turnId = identity(value.turnId, "turnId");
  const actorId = identity(value.actorId, "actorId");
  const planId = identity(value.planId, "planId");
  const planRevision = safeInteger(value.planRevision, "planRevision", 1);
  const steps = array(value.steps, "steps").map(decodeTaskStep);
  const ordered = [...steps].sort(
    (left, right) =>
      left.order - right.order || left.stepId.localeCompare(right.stepId),
  );
  if (new Set(ordered.map((step) => step.stepId)).size !== ordered.length) {
    throw invalid("TaskPlan contains duplicate step identities.");
  }

  const historyStart = optionalInteger(value.planRevisionHistoryStart, 1);
  const activeStepId = optionalIdentity(value.activeStepId);
  const plan: TaskPlan = {
    schemaVersion: safeInteger(value.schemaVersion, "schemaVersion", 1),
    actorId,
    taskId,
    turnId,
    planId,
    planRevision,
    ...(historyStart !== undefined
      ? { planRevisionHistoryStart: historyStart }
      : {}),
    planRevisions: optionalArray(value.planRevisions).map(decodeRevision),
    title: boundedString(value.title, "title", 512),
    status: closed(value.status, TASK_STATUSES, "status"),
    ...(activeStepId ? { activeStepId } : {}),
    ...optionalNamedString(value, "failureCode"),
    ...optionalNamedString(value, "safeMessage"),
    ...optionalNamedString(value, "createdAt"),
    ...optionalNamedString(value, "updatedAt"),
    ...(value.gate === undefined || value.gate === null
      ? {}
      : { gate: decodeGate(value.gate, taskId, planId, planRevision) }),
    steps: ordered,
  };
  assertTaskPlanRelationships(plan);
  return plan;
}

export function assertTaskPlanRelationships(plan: TaskPlan): void {
  const steps = new Map(plan.steps.map((step) => [step.stepId, step]));
  if (plan.activeStepId && !steps.has(plan.activeStepId)) {
    throw invalid("TaskPlan active step does not exist.");
  }
  for (const step of plan.steps) {
    if (
      new Set(step.dependsOn).size !== step.dependsOn.length ||
      step.dependsOn.some(
        (dependency) => dependency === step.stepId || !steps.has(dependency),
      )
    ) {
      throw invalid("Task step dependencies are invalid.");
    }
    if (step.guard) {
      const condition = steps.get(step.guard.conditionStepId);
      if (!condition || condition.kind !== "condition") {
        throw invalid("Task step guard does not reference a condition step.");
      }
    }
    const operation = step.operation;
    if (
      (operation?.conversationActorId &&
        operation.conversationActorId !== plan.actorId) ||
      (operation?.turnId && operation.turnId !== plan.turnId) ||
      (operation?.taskId && operation.taskId !== plan.taskId) ||
      (operation?.stepId && operation.stepId !== step.stepId)
    ) {
      throw invalid(
        "Task operation identity does not match its TaskPlan step.",
      );
    }
  }
}

export function decodeTaskStep(input: unknown): TaskStep {
  const value = record(input, "task step");
  const kind = closed(value.kind, STEP_KINDS, "step.kind");
  const source = decodeSource(value.source, kind);
  const operation =
    value.operation === undefined || value.operation === null
      ? null
      : decodeOperation(value.operation);
  const estimateRecord = optionalRecord(value.estimate);
  const estimate = estimateRecord
    ? {
        kind: closed(
          estimateRecord.kind,
          ["duration"] as const,
          "estimate.kind",
        ),
        seconds: safeInteger(estimateRecord.seconds, "estimate.seconds", 0),
      }
    : undefined;
  const approvalObservation =
    value.approvalObservation === undefined ||
    value.approvalObservation === null
      ? undefined
      : decodeApprovalObservation(value.approvalObservation);
  const guard =
    value.guard === undefined || value.guard === null
      ? undefined
      : decodeGuard(value.guard);
  const addedInPlanRevision = optionalInteger(value.addedInPlanRevision, 1);
  const cancelledInPlanRevision = optionalInteger(
    value.cancelledInPlanRevision,
    1,
  );
  const actionRequestId = optionalIdentity(value.actionRequestId);
  const approvalRequestId = optionalIdentity(value.approvalRequestId);

  return {
    stepId: identity(value.stepId, "stepId"),
    order: safeInteger(value.order, "step.order", 0),
    kind,
    status: closed(value.status, STEP_STATUSES, "step.status"),
    required: boolean(value.required, "step.required"),
    description: boundedString(value.description, "step.description", 1_024),
    source,
    mayChangeExternalState: boolean(
      value.mayChangeExternalState,
      "step.mayChangeExternalState",
    ),
    externalEffect: closed(
      value.externalEffect,
      EFFECTS,
      "step.externalEffect",
    ),
    availableActions: decodeActions(value.availableActions),
    ...optionalNamedString(value, "updatedAt"),
    ...(value.addedBy !== undefined
      ? { addedBy: closed(value.addedBy, ADDED_BY, "step.addedBy") }
      : {}),
    ...(addedInPlanRevision !== undefined ? { addedInPlanRevision } : {}),
    ...(cancelledInPlanRevision !== undefined
      ? { cancelledInPlanRevision }
      : {}),
    dependsOn: optionalArray(value.dependsOn).map((item) =>
      identity(item, "step.dependsOn"),
    ),
    ...(estimate ? { estimate } : {}),
    substeps: optionalArray(value.substeps).map(decodeSubstep),
    operation,
    ...(approvalObservation ? { approvalObservation } : {}),
    ...(guard ? { guard } : {}),
    ...(actionRequestId ? { actionRequestId } : {}),
    ...(approvalRequestId ? { approvalRequestId } : {}),
    ...optionalNamedString(value, "failureCode"),
    ...optionalNamedString(value, "safeMessage"),
    ...(typeof value.safeToSkip === "boolean"
      ? { safeToSkip: value.safeToSkip }
      : {}),
  };
}

function decodeGuard(input: unknown): NonNullable<TaskStep["guard"]> {
  const value = record(input, "step guard");
  return {
    conditionStepId: identity(value.conditionStepId, "guard.conditionStepId"),
    requiredOutcome: closed(
      value.requiredOutcome,
      ["true", "false"] as const,
      "guard.requiredOutcome",
    ),
  };
}

function decodeApprovalObservation(input: unknown): TaskApprovalObservation {
  const value = record(input, "step approval observation");
  return {
    approvalRequestId: identity(
      value.approvalRequestId,
      "approvalObservation.approvalRequestId",
    ),
    decisionMode: closed(
      value.decisionMode,
      ["unknown", "per_request", "grant"] as const,
      "approvalObservation.decisionMode",
    ),
    receiptStatus: closed(
      value.receiptStatus,
      ["approval_required", "denied"] as const,
      "approvalObservation.receiptStatus",
    ),
    observedAt: boundedString(
      value.observedAt,
      "approvalObservation.observedAt",
      128,
    ),
    ...(value.terminalOutcome !== undefined
      ? {
          terminalOutcome: closed(
            value.terminalOutcome,
            ["rejected", "expired", "timed_out"] as const,
            "approvalObservation.terminalOutcome",
          ),
        }
      : {}),
    ...optionalNamedString(value, "subjectKind"),
  };
}

function decodeGate(
  input: unknown,
  taskId: string,
  planId: string,
  planRevision: number,
): TaskPlanGate {
  const value = record(input, "plan gate");
  const requestId = optionalIdentity(value.requestId);
  const gateTaskId = optionalIdentity(value.taskId);
  const gatePlanId = optionalIdentity(value.planId);
  const gatePlanRevision = optionalInteger(value.planRevision, 1);
  const gate: TaskPlanGate = {
    mode: closed(value.mode, ["auto", "confirm"] as const, "gate.mode"),
    ...(value.status !== undefined
      ? {
          status: closed(
            value.status,
            ["pending", "satisfied", "rejected"] as const,
            "gate.status",
          ),
        }
      : {}),
    ...(requestId ? { requestId } : {}),
    ...(gateTaskId ? { taskId: gateTaskId } : {}),
    ...(gatePlanId ? { planId: gatePlanId } : {}),
    ...(gatePlanRevision !== undefined
      ? { planRevision: gatePlanRevision }
      : {}),
    ...optionalNamedString(value, "reason"),
    ...optionalNamedString(value, "decidedAt"),
  };
  if (
    gate.mode === "confirm" &&
    gate.status === "pending" &&
    (!gate.requestId ||
      gate.taskId !== taskId ||
      gate.planId !== planId ||
      gate.planRevision !== planRevision)
  ) {
    throw invalid(
      "Pending plan gate does not bind the exact TaskPlan identity.",
    );
  }
  return gate;
}

function decodeRevision(input: unknown): TaskPlan["planRevisions"][number] {
  const value = record(input, "plan revision");
  return {
    planRevision: safeInteger(value.planRevision, "revision.planRevision", 1),
    revisionCause: closed(
      value.revisionCause,
      REVISION_CAUSES,
      "revision.revisionCause",
    ),
    ...optionalNamedString(value, "committedAt"),
    addedStepIds: optionalArray(value.addedStepIds).map((item) =>
      identity(item, "revision.addedStepIds"),
    ),
    cancelledStepIds: optionalArray(value.cancelledStepIds).map((item) =>
      identity(item, "revision.cancelledStepIds"),
    ),
  };
}

function decodeSubstep(input: unknown): TaskSubstep {
  const value = record(input, "substep");
  return {
    substepId: identity(value.substepId, "substepId"),
    title: boundedString(value.title, "substep.title", 512),
    status: closed(
      value.status,
      ["running", "done", "failed"] as const,
      "substep.status",
    ),
  };
}

function decodeActions(input: unknown): TaskAvailableActions {
  const value = input === undefined ? {} : record(input, "availableActions");
  const supported = new Set<keyof TaskAvailableActions>([
    "retry",
    "skip",
    "stop",
  ]);
  if (
    Object.keys(value).some(
      (key) => !supported.has(key as keyof TaskAvailableActions),
    )
  ) {
    throw invalid("availableActions contains an unknown action verb.");
  }
  const action = (key: keyof TaskAvailableActions): boolean =>
    value[key] === undefined
      ? false
      : boolean(value[key], `availableActions.${key}`);
  return {
    retry: action("retry"),
    skip: action("skip"),
    stop: action("stop"),
  };
}

function decodeSource(
  input: unknown,
  expectedKind: TaskStepKind,
): TaskStepSource {
  const value = record(input, "step source");
  const expectedKey =
    expectedKind === "browser_action" ? "browserAction" : expectedKind;
  const knownArms = [
    "llm",
    "tool",
    "browserAction",
    "postcondition",
    "input",
    "approval",
    "web",
    "condition",
  ] as const;
  const populated = knownArms.filter((key) => optionalRecord(value[key]));
  if (populated.length !== 1 || populated[0] !== expectedKey) {
    throw invalid("Task step source does not match its kind.");
  }
  const source = record(value[expectedKey], `source.${expectedKey}`);
  switch (expectedKey) {
    case "llm":
      return { kind: "llm", label: optionalString(source.model) || "LLM" };
    case "tool": {
      const toolName = boundedString(
        source.toolName,
        "source.tool.toolName",
        256,
      );
      const serviceSlug = optionalIdentity(source.serviceSlug);
      const serviceId = optionalIdentity(source.serviceId);
      return {
        kind: "tool",
        label: toolName,
        ...(serviceSlug ? { serviceSlug } : {}),
        ...(serviceId ? { serviceId } : {}),
      };
    }
    case "browserAction":
      return {
        kind: "browserAction",
        label: optionalString(source.action) || "Browser action",
      };
    case "postcondition":
      return {
        kind: "postcondition",
        label: optionalString(source.check) || "Postcondition",
      };
    case "input":
      return { kind: "input", label: "User input" };
    case "approval":
      return { kind: "approval", label: "Approval" };
    case "condition": {
      const condition = decodeCondition(source.condition);
      return {
        kind: "condition",
        label: `${String(condition.observedValue)} >= ${String(condition.effectiveThreshold)}`,
        condition,
      };
    }
    default:
      return { kind: "web", label: "Web" };
  }
}

function decodeCondition(input: unknown): TaskNumericCondition {
  const value = record(input, "source.condition.condition");
  return {
    conditionId: identity(value.conditionId, "condition.conditionId"),
    sourceInputRequestId: identity(
      value.sourceInputRequestId,
      "condition.sourceInputRequestId",
    ),
    suggestedThreshold: integer(
      value.suggestedThreshold,
      "condition.suggestedThreshold",
    ),
    effectiveThreshold: integer(
      value.effectiveThreshold,
      "condition.effectiveThreshold",
    ),
    thresholdOrigin: closed(
      value.thresholdOrigin,
      ["suggested", "user_override"] as const,
      "condition.thresholdOrigin",
    ),
    observedValue: integer(value.observedValue, "condition.observedValue"),
    comparison: closed(
      value.comparison,
      ["gte"] as const,
      "condition.comparison",
    ),
    outcome: closed(
      value.outcome,
      ["true", "false"] as const,
      "condition.outcome",
    ),
    ...optionalNamedString(value, "evaluatedAt"),
    guardedToolName: boundedString(
      value.guardedToolName,
      "condition.guardedToolName",
      256,
    ),
  };
}

function decodeOperation(input: unknown): TaskOperation {
  const value = record(input, "operation");
  const operation: Record<string, string | number> = {};
  for (const key of [
    "conversationActorId",
    "turnId",
    "taskId",
    "stepId",
    "operationId",
  ] as const) {
    const identityValue = optionalIdentity(value[key]);
    if (identityValue) operation[key] = identityValue;
  }
  for (const key of [
    "operationGeneration",
    "latestProgressSequence",
  ] as const) {
    const numberValue = optionalInteger(value[key], 0);
    if (numberValue !== undefined) operation[key] = numberValue;
  }
  for (const key of [
    "kind",
    "phase",
    "safeMessage",
    "failureCode",
    "progressMessage",
    "startedAt",
    "updatedAt",
    "completedAt",
    "lastProgressAt",
    "stalledAt",
  ] as const) {
    const stringValue = optionalString(value[key]);
    if (stringValue) operation[key] = stringValue;
  }
  return operation;
}

function record(input: unknown, label: string): Record<string, unknown> {
  const value = optionalRecord(input);
  if (!value) throw invalid(`${label} must be an object.`);
  return value;
}

function optionalRecord(input: unknown): Record<string, unknown> | null {
  return input && typeof input === "object" && !Array.isArray(input)
    ? (input as Record<string, unknown>)
    : null;
}

function array(input: unknown, label: string): unknown[] {
  if (!Array.isArray(input)) throw invalid(`${label} must be an array.`);
  return input;
}

function optionalArray(input: unknown): unknown[] {
  return input === undefined || input === null
    ? []
    : array(input, "optional collection");
}

function identity(input: unknown, label: string): string {
  const value = optionalIdentity(input);
  if (!value) throw invalid(`${label} is invalid.`);
  return value;
}

function optionalIdentity(input: unknown): string | undefined {
  if (typeof input !== "string" || input.length < 1 || input.length > 256) {
    return undefined;
  }
  return [...input].some((character) => {
    const code = character.charCodeAt(0);
    return code <= 31 || code === 127 || /[\s/\\?#]/u.test(character);
  })
    ? undefined
    : input;
}

function boundedString(input: unknown, label: string, max: number): string {
  if (
    typeof input !== "string" ||
    input.trim() !== input ||
    input.length < 1 ||
    input.length > max
  ) {
    throw invalid(`${label} is invalid.`);
  }
  return input;
}

function optionalString(input: unknown): string | undefined {
  return typeof input === "string" && input.trim() ? input.trim() : undefined;
}

function optionalNamedString(
  value: Record<string, unknown>,
  key: string,
): Record<string, string> {
  const parsed = optionalString(value[key]);
  return parsed ? { [key]: parsed } : {};
}

function safeInteger(input: unknown, label: string, minimum: number): number {
  if (!Number.isSafeInteger(input) || (input as number) < minimum) {
    throw invalid(`${label} is invalid.`);
  }
  return input as number;
}

function integer(input: unknown, label: string): number {
  if (!Number.isSafeInteger(input)) throw invalid(`${label} is invalid.`);
  return input as number;
}

function optionalInteger(input: unknown, minimum: number): number | undefined {
  return input === undefined || input === null
    ? undefined
    : safeInteger(input, "integer", minimum);
}

function boolean(input: unknown, label: string): boolean {
  if (typeof input !== "boolean") throw invalid(`${label} is invalid.`);
  return input;
}

function closed<const T extends readonly string[]>(
  input: unknown,
  values: T,
  label: string,
): T[number] {
  if (typeof input !== "string" || !values.includes(input)) {
    throw invalid(`${label} is outside the closed contract.`);
  }
  return input as T[number];
}

function invalid(message: string): TaskPlanProtocolError {
  return new TaskPlanProtocolError(message);
}
