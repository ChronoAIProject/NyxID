export type ChatTaskStatus =
  | "active"
  | "succeeded"
  | "failed"
  | "stopped"
  | "blocked";

export type ChatTaskStepStatus =
  | "planned"
  | "waiting"
  | "running"
  | "done"
  | "failed"
  | "skipped"
  | "cancelled"
  | "uncertain";

export type ChatExternalEffect =
  | "not_started"
  | "not_applied"
  | "confirmed"
  | "may_have_changed";

export type ChatTaskStepKind =
  | "llm"
  | "tool"
  | "browser_action"
  | "postcondition"
  | "input"
  | "approval"
  | "web"
  | "condition";

export type ChatPlanRevisionCause =
  | "initial"
  | "scope_resolution"
  | "failure_recovery"
  | "steering"
  | "user_revision";

export type ChatStepAddedBy =
  | "initial"
  | "scope_resolution"
  | "replan"
  | "steering"
  | "user_revision";

export type ChatAvailableActions = {
  readonly retry: boolean;
  readonly skip: boolean;
  readonly stop: boolean;
};

export type ChatTaskSubstep = {
  readonly substepId: string;
  readonly title: string;
  readonly status: "running" | "done" | "failed";
};

export type ChatTaskStepSource =
  | { readonly kind: "llm"; readonly label: string }
  | {
      readonly kind: "tool";
      readonly label: string;
      readonly serviceSlug?: string;
      readonly serviceId?: string;
    }
  | { readonly kind: "browserAction"; readonly label: string }
  | { readonly kind: "postcondition"; readonly label: string }
  | { readonly kind: "input"; readonly label: string }
  | { readonly kind: "approval"; readonly label: string }
  | { readonly kind: "web"; readonly label: string }
  | {
      readonly kind: "condition";
      readonly label: string;
      readonly condition: ChatNumericCondition;
    };

export type ChatNumericCondition = {
  readonly conditionId: string;
  readonly sourceInputRequestId: string;
  readonly suggestedThreshold: number;
  readonly effectiveThreshold: number;
  readonly thresholdOrigin: "suggested" | "user_override";
  readonly observedValue: number;
  readonly comparison: "gte";
  readonly outcome: "true" | "false";
  readonly evaluatedAt?: string;
  readonly guardedToolName: string;
};

export type ChatStepGuard = {
  readonly conditionStepId: string;
  readonly requiredOutcome: "true" | "false";
};

export type ChatTaskOperation = {
  readonly conversationActorId?: string;
  readonly turnId?: string;
  readonly taskId?: string;
  readonly stepId?: string;
  readonly operationId?: string;
  readonly operationGeneration?: number;
  readonly kind?: string;
  readonly phase?: string;
  readonly latestProgressSequence?: number;
  readonly safeMessage?: string;
  readonly failureCode?: string;
  readonly progressMessage?: string;
  readonly startedAt?: string;
  readonly updatedAt?: string;
  readonly completedAt?: string;
  readonly lastProgressAt?: string;
  readonly stalledAt?: string;
};

export type ChatApprovalObservation = {
  readonly approvalRequestId: string;
  readonly decisionMode: "unknown" | "per_request" | "grant";
  readonly receiptStatus: "approval_required" | "denied";
  readonly observedAt: string;
  readonly terminalOutcome?: "rejected" | "expired" | "timed_out";
  readonly subjectKind?: string;
};

export type ChatActorStep = {
  readonly stepId: string;
  readonly order: number;
  readonly kind: ChatTaskStepKind;
  readonly status: ChatTaskStepStatus;
  readonly required: boolean;
  readonly description: string;
  readonly source: ChatTaskStepSource;
  readonly mayChangeExternalState: boolean;
  readonly externalEffect: ChatExternalEffect;
  readonly availableActions: ChatAvailableActions;
  readonly updatedAt?: string;
  readonly addedBy?: ChatStepAddedBy;
  readonly addedInPlanRevision?: number;
  readonly cancelledInPlanRevision?: number;
  readonly dependsOn: readonly string[];
  readonly estimate?: { readonly kind: "duration"; readonly seconds: number };
  readonly substeps: readonly ChatTaskSubstep[];
  readonly operation?: ChatTaskOperation | null;
  readonly approvalObservation?: ChatApprovalObservation;
  readonly guard?: ChatStepGuard;
  readonly actionRequestId?: string;
  readonly approvalRequestId?: string;
  readonly failureCode?: string;
  readonly safeMessage?: string;
  readonly safeToSkip?: boolean;
};

export type ChatPlanGate = {
  readonly mode: "auto" | "confirm";
  readonly status?: "pending" | "satisfied" | "rejected";
  readonly reason?: string;
  readonly decidedAt?: string;
};

export type ChatPlanRevision = {
  readonly planRevision: number;
  readonly revisionCause: ChatPlanRevisionCause;
  readonly committedAt?: string;
  readonly addedStepIds: readonly string[];
  readonly cancelledStepIds: readonly string[];
};

export type ChatTaskPlan = {
  readonly schemaVersion: number;
  readonly actorId: string;
  readonly taskId: string;
  readonly turnId: string;
  readonly planId: string;
  readonly planRevision: number;
  readonly planRevisionHistoryStart?: number;
  readonly planRevisions: readonly ChatPlanRevision[];
  readonly title: string;
  readonly status: ChatTaskStatus;
  readonly activeStepId?: string;
  readonly failureCode?: string;
  readonly safeMessage?: string;
  readonly createdAt?: string;
  readonly updatedAt?: string;
  readonly gate?: ChatPlanGate | null;
  readonly steps: readonly ChatActorStep[];
};

export class ChatTaskPlanProtocolError extends Error {
  readonly code = "NYXID_TASK_PLAN_INVALID";

  constructor(message: string) {
    super(message);
    this.name = "ChatTaskPlanProtocolError";
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

export function decodeChatTaskPlan(input: unknown): ChatTaskPlan {
  const value = record(input, "task plan");
  const taskId = identity(value.taskId, "taskId");
  const turnId = identity(value.turnId, "turnId");
  const actorId = identity(value.actorId, "actorId");
  const planId = identity(value.planId, "planId");
  const planRevision = safeInteger(value.planRevision, "planRevision", 1);
  const steps = array(value.steps, "steps").map(decodeChatTaskStep);
  const ordered = [...steps].sort(
    (left, right) =>
      left.order - right.order || left.stepId.localeCompare(right.stepId),
  );
  if (new Set(ordered.map((step) => step.stepId)).size !== ordered.length) {
    throw invalid("TaskPlan contains duplicate step identities.");
  }
  return {
    schemaVersion: safeInteger(value.schemaVersion, "schemaVersion", 1),
    actorId,
    taskId,
    turnId,
    planId,
    planRevision,
    ...(optionalInteger(value.planRevisionHistoryStart, 1) !== undefined
      ? {
          planRevisionHistoryStart: optionalInteger(
            value.planRevisionHistoryStart,
            1,
          ),
        }
      : {}),
    planRevisions: optionalArray(value.planRevisions).map(decodeRevision),
    title: boundedString(value.title, "title", 512),
    status: closed(value.status, TASK_STATUSES, "status"),
    ...(optionalIdentity(value.activeStepId)
      ? { activeStepId: optionalIdentity(value.activeStepId) }
      : {}),
    ...(optionalString(value.failureCode)
      ? { failureCode: optionalString(value.failureCode) }
      : {}),
    ...(optionalString(value.safeMessage)
      ? { safeMessage: optionalString(value.safeMessage) }
      : {}),
    ...(optionalString(value.createdAt)
      ? { createdAt: optionalString(value.createdAt) }
      : {}),
    ...(optionalString(value.updatedAt)
      ? { updatedAt: optionalString(value.updatedAt) }
      : {}),
    ...(value.gate === undefined || value.gate === null
      ? {}
      : { gate: decodeGate(value.gate) }),
    steps: ordered,
  };
}

export function decodeChatTaskStep(input: unknown): ChatActorStep {
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
  return {
    stepId: identity(value.stepId, "stepId"),
    order: safeInteger(value.order, "step.order", 0),
    kind,
    status: closed(value.status, STEP_STATUSES, "step.status"),
    required: boolean(value.required, "step.required"),
    description: boundedString(value.description, "step.description", 1024),
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
    ...(optionalString(value.updatedAt)
      ? { updatedAt: optionalString(value.updatedAt) }
      : {}),
    ...(value.addedBy !== undefined
      ? { addedBy: closed(value.addedBy, ADDED_BY, "step.addedBy") }
      : {}),
    ...(optionalInteger(value.addedInPlanRevision, 1) !== undefined
      ? { addedInPlanRevision: optionalInteger(value.addedInPlanRevision, 1) }
      : {}),
    ...(optionalInteger(value.cancelledInPlanRevision, 1) !== undefined
      ? {
          cancelledInPlanRevision: optionalInteger(
            value.cancelledInPlanRevision,
            1,
          ),
        }
      : {}),
    dependsOn: optionalArray(value.dependsOn).map((item) =>
      identity(item, "step.dependsOn"),
    ),
    ...(estimate ? { estimate } : {}),
    substeps: optionalArray(value.substeps).map(decodeSubstep),
    operation,
    ...(approvalObservation ? { approvalObservation } : {}),
    ...(guard ? { guard } : {}),
    ...(optionalIdentity(value.actionRequestId)
      ? { actionRequestId: optionalIdentity(value.actionRequestId) }
      : {}),
    ...(optionalIdentity(value.approvalRequestId)
      ? { approvalRequestId: optionalIdentity(value.approvalRequestId) }
      : {}),
    ...(optionalString(value.failureCode)
      ? { failureCode: optionalString(value.failureCode) }
      : {}),
    ...(optionalString(value.safeMessage)
      ? { safeMessage: optionalString(value.safeMessage) }
      : {}),
    ...(typeof value.safeToSkip === "boolean"
      ? { safeToSkip: value.safeToSkip }
      : {}),
  };
}

function decodeGuard(input: unknown): ChatStepGuard {
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

function decodeApprovalObservation(input: unknown): ChatApprovalObservation {
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
    ...(optionalString(value.subjectKind)
      ? { subjectKind: optionalString(value.subjectKind) }
      : {}),
  };
}

function decodeGate(input: unknown): ChatPlanGate {
  const value = record(input, "plan gate");
  return {
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
    ...(optionalString(value.reason)
      ? { reason: optionalString(value.reason) }
      : {}),
    ...(optionalString(value.decidedAt)
      ? { decidedAt: optionalString(value.decidedAt) }
      : {}),
  };
}

function decodeRevision(input: unknown): ChatPlanRevision {
  const value = record(input, "plan revision");
  return {
    planRevision: safeInteger(value.planRevision, "revision.planRevision", 1),
    revisionCause: closed(
      value.revisionCause,
      REVISION_CAUSES,
      "revision.revisionCause",
    ),
    ...(optionalString(value.committedAt)
      ? { committedAt: optionalString(value.committedAt) }
      : {}),
    addedStepIds: optionalArray(value.addedStepIds).map((item) =>
      identity(item, "revision.addedStepIds"),
    ),
    cancelledStepIds: optionalArray(value.cancelledStepIds).map((item) =>
      identity(item, "revision.cancelledStepIds"),
    ),
  };
}

function decodeSubstep(input: unknown): ChatTaskSubstep {
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

function decodeActions(input: unknown): ChatAvailableActions {
  const value = input === undefined ? {} : record(input, "availableActions");
  const action = (key: keyof ChatAvailableActions): boolean =>
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
  expectedKind: ChatTaskStepKind,
): ChatTaskStepSource {
  const value = record(input, "step source");
  const expectedKey =
    expectedKind === "browser_action" ? "browserAction" : expectedKind;
  const populated = Object.keys(value).filter((key) =>
    optionalRecord(value[key]),
  );
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
        label: optionalString(source.action) || "",
      };
    case "postcondition":
      return {
        kind: "postcondition",
        label: optionalString(source.check) || "",
      };
    case "input":
      return { kind: "input", label: "" };
    case "approval":
      return { kind: "approval", label: "" };
    case "condition": {
      const condition = decodeCondition(source.condition);
      return {
        kind: "condition",
        label: `${condition.observedValue} >= ${condition.effectiveThreshold}`,
        condition,
      };
    }
    default:
      return { kind: "web", label: "Web" };
  }
}

function decodeCondition(input: unknown): ChatNumericCondition {
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
    ...(optionalString(value.evaluatedAt)
      ? { evaluatedAt: optionalString(value.evaluatedAt) }
      : {}),
    guardedToolName: boundedString(
      value.guardedToolName,
      "condition.guardedToolName",
      256,
    ),
  };
}

function decodeOperation(input: unknown): ChatTaskOperation {
  const value = record(input, "operation");
  const operation: ChatTaskOperation = {};
  for (const key of [
    "conversationActorId",
    "turnId",
    "taskId",
    "stepId",
    "operationId",
  ] as const) {
    const identityValue = optionalIdentity(value[key]);
    if (identityValue) Object.assign(operation, { [key]: identityValue });
  }
  for (const key of [
    "operationGeneration",
    "latestProgressSequence",
  ] as const) {
    const numberValue = optionalInteger(value[key], 0);
    if (numberValue !== undefined)
      Object.assign(operation, { [key]: numberValue });
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
    if (stringValue) Object.assign(operation, { [key]: stringValue });
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
  if (typeof input !== "string" || input.length < 1 || input.length > 256)
    return undefined;
  return [...input].some(
    (character) =>
      character.charCodeAt(0) <= 31 ||
      character.charCodeAt(0) === 127 ||
      /[\s/\\?#]/u.test(character),
  )
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

function safeInteger(input: unknown, label: string, minimum: number): number {
  if (!Number.isSafeInteger(input) || (input as number) < minimum)
    throw invalid(`${label} is invalid.`);
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
  if (typeof input !== "string" || !values.includes(input))
    throw invalid(`${label} is outside the closed contract.`);
  return input as T[number];
}

function invalid(message: string): ChatTaskPlanProtocolError {
  return new ChatTaskPlanProtocolError(message);
}
