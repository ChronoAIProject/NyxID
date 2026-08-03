import { composeUnreportedCompletedNote } from "@/lib/assistant/action-notes";
import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import { toTerminalBlock } from "@/lib/assistant/stream";
import {
  ACTION_SCHEMA_VERSION,
  actionReportSchema,
  assistantActionRequestSchema,
  type ActionReport,
} from "@/schemas/assistant-actions";
import type {
  ActionCardContentBlock,
  ActionCardStatus,
  ApprovalCardContentBlock,
  ContentBlock,
  RunContentBlock,
  TurnEvent,
  TurnHandle,
} from "@/types/assistant";

export const EVENT_CADENCE_MS = 100;
const MAX_WAIT_MS = 10_000;
const TEXT_CHUNK_SIZE = 40;
const APPROVAL_TTL_MS = 15 * 60 * 1_000;

export interface WorldPort {
  isConnected(serviceSlug: string): boolean;
  connect(serviceSlug: string): void;
  disconnect(serviceSlug: string): void;
}

export interface CursorSource {
  next(conversationId: string): number;
}

export interface ScenarioActionTarget {
  readonly service?: string;
  readonly scopes?: readonly string[];
  readonly custom?: {
    readonly name: string;
    readonly endpointUrl: string;
    readonly authMethod?:
      | "bearer"
      | "header"
      | "query"
      | "path"
      | "basic"
      | "body"
      | "none";
    readonly authKeyName?: string;
  };
}

export interface ScenarioApprovalOptions {
  readonly approvalMode?: "per_request" | "grant";
  readonly grantDurationSec?: number | null;
}

export type AwaitDisposition =
  | "completed"
  | "declined"
  | "failed"
  | "approved"
  | "denied"
  | "wake";

export type AwaitBranches = Partial<
  Record<
    AwaitDisposition,
    (script: ScenarioScriptBuilder) => ScenarioScriptBuilder
  >
>;

type ScenarioStep =
  | { readonly kind: "say"; readonly markdown: string }
  | {
      readonly kind: "tool";
      readonly label: string;
      readonly result: string;
      readonly failed: boolean;
    }
  | {
      readonly kind: "action";
      readonly action: string;
      readonly target: ScenarioActionTarget;
    }
  | {
      readonly kind: "approval";
      readonly serviceSlug: string;
      readonly body: string;
      readonly options: ScenarioApprovalOptions;
    }
  | {
      readonly kind: "artifact";
      readonly name: string;
      readonly preview: string;
      readonly mime: string;
    }
  | {
      readonly kind: "await";
      readonly branches: Readonly<
        Partial<Record<AwaitDisposition, readonly ScenarioStep[]>>
      >;
    }
  | {
      readonly kind: "need";
      readonly serviceSlug: string;
      readonly flowName: string;
    }
  | { readonly kind: "run"; readonly flowName: string }
  | {
      readonly kind: "when";
      readonly connected: boolean;
      readonly serviceSlug: string;
      readonly steps: readonly ScenarioStep[];
    }
  | { readonly kind: "connect"; readonly serviceSlug: string }
  | { readonly kind: "disconnect"; readonly serviceSlug: string }
  | { readonly kind: "wait"; readonly milliseconds: number }
  | { readonly kind: "stop" }
  | { readonly kind: "fail"; readonly code: string; readonly message: string };

export class ScenarioScriptBuilder {
  readonly steps: readonly ScenarioStep[];

  constructor(steps: readonly ScenarioStep[] = []) {
    this.steps = steps;
  }

  private append(step: ScenarioStep): ScenarioScriptBuilder {
    return new ScenarioScriptBuilder([...this.steps, step]);
  }

  say(markdown: string): ScenarioScriptBuilder {
    return this.append({ kind: "say", markdown });
  }

  tool(label: string, result: string): ScenarioScriptBuilder {
    return this.append({ kind: "tool", label, result, failed: false });
  }

  toolFail(label: string, error: string): ScenarioScriptBuilder {
    return this.append({ kind: "tool", label, result: error, failed: true });
  }

  action(action: string, target: ScenarioActionTarget): ScenarioScriptBuilder {
    return this.append({ kind: "action", action, target });
  }

  approval(
    serviceSlug: string,
    body: string,
    options: ScenarioApprovalOptions = {},
  ): ScenarioScriptBuilder {
    return this.append({ kind: "approval", serviceSlug, body, options });
  }

  artifact(
    name: string,
    preview: string,
    mime = "text/plain",
  ): ScenarioScriptBuilder {
    return this.append({ kind: "artifact", name, preview, mime });
  }

  await(branches: AwaitBranches = {}): ScenarioScriptBuilder {
    const compiledBranches = Object.fromEntries(
      Object.entries(branches).map(([disposition, build]) => [
        disposition,
        build(new ScenarioScriptBuilder()).steps,
      ]),
    ) as Partial<Record<AwaitDisposition, readonly ScenarioStep[]>>;
    return this.append({ kind: "await", branches: compiledBranches });
  }

  need(serviceSlug: string, flowName: string): ScenarioScriptBuilder {
    return this.append({ kind: "need", serviceSlug, flowName });
  }

  run(flowName: string): ScenarioScriptBuilder {
    return this.append({ kind: "run", flowName });
  }

  whenConnected(
    serviceSlug: string,
    build: (script: ScenarioScriptBuilder) => ScenarioScriptBuilder,
  ): ScenarioScriptBuilder {
    return this.append({
      kind: "when",
      connected: true,
      serviceSlug,
      steps: build(new ScenarioScriptBuilder()).steps,
    });
  }

  whenNotConnected(
    serviceSlug: string,
    build: (script: ScenarioScriptBuilder) => ScenarioScriptBuilder,
  ): ScenarioScriptBuilder {
    return this.append({
      kind: "when",
      connected: false,
      serviceSlug,
      steps: build(new ScenarioScriptBuilder()).steps,
    });
  }

  connect(serviceSlug: string): ScenarioScriptBuilder {
    return this.append({ kind: "connect", serviceSlug });
  }

  disconnect(serviceSlug: string): ScenarioScriptBuilder {
    return this.append({ kind: "disconnect", serviceSlug });
  }

  wait(milliseconds: number): ScenarioScriptBuilder {
    return this.append({
      kind: "wait",
      milliseconds: Math.min(MAX_WAIT_MS, Math.max(0, milliseconds)),
    });
  }

  stop(): ScenarioScriptBuilder {
    return this.append({ kind: "stop" });
  }

  fail(code: string, message: string): ScenarioScriptBuilder {
    return this.append({ kind: "fail", code, message });
  }
}

export interface ScenarioFlow {
  readonly steps: readonly ScenarioStep[];
}

export interface ScenarioDefinition {
  readonly id: string;
  readonly pattern: RegExp;
  readonly build: (
    script: ScenarioScriptBuilder,
    match: RegExpMatchArray,
  ) => ScenarioScriptBuilder;
}

export function flow(
  build: (script: ScenarioScriptBuilder) => ScenarioScriptBuilder,
): ScenarioFlow {
  return { steps: build(new ScenarioScriptBuilder()).steps };
}

export function scenario(
  id: string,
  pattern: RegExp,
  build: ScenarioDefinition["build"],
): ScenarioDefinition {
  return { id, pattern, build };
}

export class ScenarioConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ScenarioConfigError";
  }
}

export interface CompiledScenarioSet {
  readonly scenarios: readonly ScenarioDefinition[];
  readonly flows: Readonly<Record<string, ScenarioFlow>>;
}

function fakeMatch(): RegExpMatchArray {
  const match = ["", "capture"] as unknown as RegExpMatchArray;
  match.index = 0;
  match.input = "";
  return match;
}

function referencesIn(steps: readonly ScenarioStep[]): readonly string[] {
  return steps.flatMap((step): readonly string[] => {
    if (step.kind === "run" || step.kind === "need") return [step.flowName];
    if (step.kind === "when") return referencesIn(step.steps);
    if (step.kind === "await") {
      return Object.values(step.branches).flatMap((branch) =>
        referencesIn(branch ?? []),
      );
    }
    return [];
  });
}

function validateActionStep(
  step: Extract<ScenarioStep, { kind: "action" }>,
): void {
  const hasCatalog = typeof step.target.service === "string";
  const hasCustom = step.target.custom !== undefined;
  if (hasCatalog === hasCustom) {
    throw new ScenarioConfigError(
      `Action ${step.action} must specify exactly one catalog or custom target.`,
    );
  }
  const request = assistantActionRequestSchema.parse({
    schemaVersion: ACTION_SCHEMA_VERSION,
    actorId: "mockchat-config",
    originTurnId: "mockchat-config-turn",
    taskId: "mockchat-config-task",
    stepId: "mockchat-config-step",
    actionRequestId: "mockchat-config-action",
    action: step.action,
    params: hasCatalog
      ? {
          catalogService: {
            serviceSlug: step.target.service,
            requestedScopes: step.target.scopes ?? [],
          },
        }
      : {
          customService: {
            name: step.target.custom?.name,
            endpointUrl: step.target.custom?.endpointUrl,
            authMethod: step.target.custom?.authMethod,
            authKeyName: step.target.custom?.authKeyName,
          },
        },
  });
  if (!resolveAssistantAction(request).supported) {
    throw new ScenarioConfigError(`Action ${step.action} is not supported.`);
  }
}

function validateSteps(
  steps: readonly ScenarioStep[],
  flows: Readonly<Record<string, ScenarioFlow>>,
  resumableCards = 0,
): number {
  let cardCount = resumableCards;
  for (const step of steps) {
    if (step.kind === "action") {
      validateActionStep(step);
      cardCount += 1;
      continue;
    }
    if (step.kind === "approval") {
      if (!step.serviceSlug.trim() || !step.body.trim()) {
        throw new ScenarioConfigError(
          "Approval cards require a service and body.",
        );
      }
      cardCount += 1;
      continue;
    }
    if (step.kind === "run" || step.kind === "need") {
      const referenced = flows[step.flowName];
      if (!referenced) {
        throw new ScenarioConfigError(
          `Unknown flow reference: ${step.flowName}`,
        );
      }
      cardCount = Math.max(
        cardCount,
        validateSteps(referenced.steps, flows, cardCount),
      );
      continue;
    }
    if (step.kind === "when") {
      cardCount = Math.max(
        cardCount,
        validateSteps(step.steps, flows, cardCount),
      );
      continue;
    }
    if (step.kind === "await") {
      if (cardCount > 1) {
        throw new ScenarioConfigError(
          "A scenario segment cannot park more than one resumable card (F10).",
        );
      }
      for (const branch of Object.values(step.branches)) {
        validateSteps(branch ?? [], flows, 0);
      }
      cardCount = 0;
    }
  }
  return cardCount;
}

export function compileScenarioSet(
  scenarios: readonly ScenarioDefinition[],
  flows: Readonly<Record<string, ScenarioFlow>>,
): CompiledScenarioSet {
  const ids = new Set<string>();
  for (const [flowName, definition] of Object.entries(flows)) {
    const references = referencesIn(definition.steps);
    for (const reference of references) {
      if (!flows[reference]) {
        throw new ScenarioConfigError(`Unknown flow reference: ${reference}`);
      }
      if (reference === flowName) {
        throw new ScenarioConfigError(
          `Flow recursion is not allowed: ${flowName}`,
        );
      }
      throw new ScenarioConfigError(
        `Flows cannot reference other flows in v1: ${flowName} -> ${reference}`,
      );
    }
    validateSteps(definition.steps, flows);
  }
  for (const definition of scenarios) {
    if (!definition.id.trim())
      throw new ScenarioConfigError("Scenario id is required.");
    if (ids.has(definition.id)) {
      throw new ScenarioConfigError(`Duplicate scenario id: ${definition.id}`);
    }
    ids.add(definition.id);
    let steps: readonly ScenarioStep[];
    try {
      steps = definition.build(new ScenarioScriptBuilder(), fakeMatch()).steps;
      validateSteps(steps, flows);
    } catch (error) {
      if (error instanceof ScenarioConfigError) throw error;
      const detail = error instanceof Error ? error.message : "invalid config";
      throw new ScenarioConfigError(`Scenario ${definition.id}: ${detail}`);
    }
  }
  return { scenarios: [...scenarios], flows: { ...flows } };
}

export interface ScenarioMatch {
  readonly scenarioId: string;
  readonly steps: readonly ScenarioStep[];
}

export interface ScenarioRunHooks {
  readonly onPark?: (requestId: string) => void;
  readonly onTerminal?: (
    status: "blocked" | "completed" | "failed" | "cancelled",
  ) => void;
}

export type ActionVerification = "verified" | "mismatch" | "unverified";

export interface PreparedActionContinuation {
  readonly expectedServiceSlug: string | null;
  readonly report: ActionReport;
  cancel(): void;
  resume(
    verification: ActionVerification,
    hooks?: ScenarioRunHooks,
  ): TurnHandle | null;
}

interface Continuation {
  readonly conversationId: string;
  readonly originTurnId: string;
  readonly requestId: string;
  readonly blockId: string | null;
  readonly cardKind: "action" | "approval" | "wake";
  readonly branches: Readonly<
    Partial<Record<AwaitDisposition, readonly ScenarioStep[]>>
  >;
  readonly remaining: readonly ScenarioStep[];
  readonly emit: (event: TurnEvent) => void;
  readonly expiresAt: number | null;
}

interface PlannedContinuation {
  readonly requestId: string;
  readonly blockId: string | null;
  readonly cardKind: Continuation["cardKind"];
  readonly branches: Continuation["branches"];
  readonly remaining: readonly ScenarioStep[];
  readonly expiresAt: number | null;
}

interface RunningScript {
  readonly conversationId: string;
  readonly turnId: string;
  readonly messageId: string | null;
  readonly emit: (event: TurnEvent) => void;
  readonly hooks: ScenarioRunHooks;
  readonly timers: Set<ReturnType<typeof setTimeout>>;
  readonly openBlocks: Map<string, ContentBlock>;
  readonly continuation: PlannedContinuation | null;
  messageStarted: boolean;
  messageCompleted: boolean;
  cancelled: boolean;
  finished: boolean;
}

interface PlanItem {
  readonly delayMs: number;
  readonly event?: UnstampedTurnEvent;
  readonly effect?: () => void;
}

type WithoutCursor<T> = T extends unknown ? Omit<T, "cursor"> : never;
type UnstampedTurnEvent = WithoutCursor<TurnEvent>;

interface SegmentPlan {
  readonly items: readonly PlanItem[];
  readonly messageId: string | null;
  readonly continuation: PlannedContinuation | null;
}

interface RuntimeOptions {
  readonly now?: () => number;
  readonly cadenceMs?: number;
}

function continuationKey(conversationId: string, requestId: string): string {
  return `${conversationId}\u0000${requestId}`;
}

function chunkText(text: string): readonly string[] {
  if (!text) return [""];
  const chunks: string[] = [];
  for (let index = 0; index < text.length; index += TEXT_CHUNK_SIZE) {
    chunks.push(text.slice(index, index + TEXT_CHUNK_SIZE));
  }
  return chunks;
}

function actionOutcome(disposition: ActionReport["disposition"]): {
  readonly status: ActionCardStatus;
  readonly note: string;
} {
  if (disposition === "completed") {
    return {
      status: "completed",
      note: "Reported — awaiting assistant verification.",
    };
  }
  if (disposition === "declined") {
    return {
      status: "declined",
      note: "You declined this request. No service was connected and no credential was shared.",
    };
  }
  return {
    status: "failed",
    note: "The connection could not be completed. Ask the assistant to request it again.",
  };
}

export class ScenarioEngine {
  private readonly running = new Map<string, RunningScript>();
  private readonly continuations = new Map<string, Continuation>();
  private readonly cards = new Map<string, Map<string, ContentBlock>>();
  private readonly internalCursors = new Map<string, number>();
  private sequence = 0;
  private readonly now: () => number;
  private readonly cadenceMs: number;
  private readonly compiled: CompiledScenarioSet;
  private readonly world: WorldPort;
  private readonly cursorSource: CursorSource;

  constructor(
    compiled: CompiledScenarioSet,
    world: WorldPort,
    cursorSource?: CursorSource,
    options: RuntimeOptions = {},
  ) {
    this.compiled = compiled;
    this.world = world;
    this.cursorSource = cursorSource ?? {
      next: (conversationId) => {
        const cursor = (this.internalCursors.get(conversationId) ?? 0) + 1;
        this.internalCursors.set(conversationId, cursor);
        return cursor;
      },
    };
    this.now = options.now ?? Date.now;
    this.cadenceMs = options.cadenceMs ?? EVENT_CADENCE_MS;
  }

  match(
    content: string,
    disabledScenarioIds: readonly string[] = [],
  ): ScenarioMatch | null {
    const disabled = new Set(disabledScenarioIds);
    for (const definition of this.compiled.scenarios) {
      if (disabled.has(definition.id)) continue;
      definition.pattern.lastIndex = 0;
      const match = definition.pattern.exec(content);
      definition.pattern.lastIndex = 0;
      if (!match) continue;
      const steps = definition.build(new ScenarioScriptBuilder(), match).steps;
      validateSteps(steps, this.compiled.flows);
      return { scenarioId: definition.id, steps };
    }
    return null;
  }

  play(
    conversationId: string,
    match: ScenarioMatch,
    onEvent: (event: TurnEvent) => void,
    hooks: ScenarioRunHooks = {},
  ): TurnHandle {
    return this.startSegment(conversationId, match.steps, onEvent, true, hooks);
  }

  isRunning(conversationId: string): boolean {
    return this.running.has(conversationId);
  }

  hasContinuation(conversationId: string): boolean {
    return [...this.continuations.values()].some(
      (continuation) => continuation.conversationId === conversationId,
    );
  }

  findBlock(conversationId: string, blockId: string): ContentBlock | null {
    return this.cards.get(conversationId)?.get(blockId) ?? null;
  }

  setActionCardInProgress(
    conversationId: string,
    blockId: string,
    inProgress: boolean,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): void {
    const card = this.requireActionCard(conversationId, blockId);
    if (
      card.status === "blocked" ||
      card.status === "completed" ||
      card.status === "conflicted" ||
      card.status === "declined" ||
      card.status === "failed" ||
      card.status === "unsupported"
    ) {
      return;
    }
    this.emitLocalPatch(
      conversationId,
      blockId,
      { status: inProgress ? "in_progress" : "pending", outcome_note: "" },
      onEvent,
    );
  }

  blockActionCard(
    conversationId: string,
    blockId: string,
    note: string,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): void {
    const card = this.requireActionCard(conversationId, blockId);
    if (
      card.status === "completed" ||
      card.status === "conflicted" ||
      card.status === "declined" ||
      card.status === "failed"
    ) {
      return;
    }
    this.emitLocalPatch(
      conversationId,
      blockId,
      { status: "blocked", outcome_note: note },
      onEvent,
    );
  }

  prepareActionContinuation(
    conversationId: string,
    originTurnId: string,
    reports: readonly ActionReport[],
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): PreparedActionContinuation {
    const validated = reports.map((report) => actionReportSchema.parse(report));
    if (validated.length !== 1) {
      throw new Error("Mock scenarios require exactly one action report.");
    }
    const report = validated[0];
    if (!report || report.originTurnId !== originTurnId) {
      throw new Error("Action report origin does not match the continuation.");
    }
    const key = continuationKey(conversationId, report.actionRequestId);
    const continuation = this.continuations.get(key);
    if (!continuation || continuation.originTurnId !== originTurnId) {
      throw new Error("Action continuation was not found.");
    }
    const card = continuation.blockId
      ? this.requireActionCard(conversationId, continuation.blockId)
      : null;
    if (!card) throw new Error("Action request was not found.");
    const refusedByCardState =
      card.status === "conflicted" ||
      (card.status === "blocked" && report.disposition === "completed");
    if (refusedByCardState) {
      if (report.disposition === "completed" && report.resource) {
        this.emitLocalPatch(
          conversationId,
          card.block_id,
          {
            outcome_note: composeUnreportedCompletedNote(
              card.status,
              card.outcome_note,
            ),
          },
          onEvent,
        );
      }
      throw new Error(
        "This action request can no longer be continued from the current card state.",
      );
    }
    const initialOutcome = actionOutcome(report.disposition);
    this.emitLocalPatch(
      conversationId,
      card.block_id,
      { status: initialOutcome.status, outcome_note: initialOutcome.note },
      onEvent,
    );

    let settled = false;
    return {
      expectedServiceSlug:
        card.params.variant === "catalog" ? card.params.service_slug : null,
      report,
      cancel: () => {
        if (settled) return;
        settled = true;
        this.continuations.delete(key);
      },
      resume: (verification, hooks = {}) => {
        if (settled) return null;
        settled = true;
        this.continuations.delete(key);
        let disposition: AwaitDisposition =
          report.disposition === "completed"
            ? "completed"
            : report.disposition === "declined"
              ? "declined"
              : "failed";
        let allowConnect = false;
        if (report.disposition === "completed") {
          if (verification === "verified") {
            allowConnect = true;
          } else if (verification === "mismatch") {
            disposition = "failed";
            this.emitLocalPatch(
              conversationId,
              card.block_id,
              {
                status: "failed",
                outcome_note: "A different service was connected.",
              },
              onEvent,
            );
          } else {
            this.emitLocalPatch(
              conversationId,
              card.block_id,
              {
                status: "completed",
                outcome_note:
                  "The connection was reported, but could not be verified.",
              },
              onEvent,
            );
          }
        }
        return this.resumeContinuation(
          continuation,
          disposition,
          onEvent,
          allowConnect,
          hooks,
        );
      },
    };
  }

  decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
    onEvent: (event: TurnEvent) => void = () => undefined,
    hooks: ScenarioRunHooks = {},
  ): TurnHandle | null {
    const card = this.cards.get(conversationId)?.get(blockId);
    if (card?.type !== "approval_card") {
      throw new Error("Approval request was not found.");
    }
    if (card.decision !== null) {
      throw new Error("Approval request was already decided.");
    }
    const key = continuationKey(conversationId, card.approval_request_id);
    const continuation = this.continuations.get(key);
    if (!continuation) throw new Error("Approval continuation was not found.");
    if (
      continuation.expiresAt !== null &&
      this.now() > continuation.expiresAt
    ) {
      this.emitLocalPatch(
        conversationId,
        blockId,
        { decision: "expired" },
        onEvent,
      );
      this.continuations.delete(key);
      return null;
    }
    this.emitLocalPatch(
      conversationId,
      blockId,
      {
        decision: approved ? "approved" : "denied",
        decision_channel: "web",
      },
      onEvent,
    );
    this.continuations.delete(key);
    return this.resumeContinuation(
      continuation,
      approved ? "approved" : "denied",
      onEvent,
      false,
      hooks,
    );
  }

  wake(
    conversationId: string,
    originTurnId: string,
    onEvent: (event: TurnEvent) => void = () => undefined,
    hooks: ScenarioRunHooks = {},
  ): TurnHandle {
    const continuation = [...this.continuations.values()].find(
      (candidate) =>
        candidate.conversationId === conversationId &&
        candidate.originTurnId === originTurnId,
    );
    if (!continuation) throw new Error("Action continuation was not found.");
    this.continuations.delete(
      continuationKey(conversationId, continuation.requestId),
    );
    const wakeSteps = continuation.branches.wake ?? [
      { kind: "say", markdown: "The assistant resumed the blocked turn." },
    ];
    return this.startSegment(
      conversationId,
      [...wakeSteps, ...continuation.remaining],
      onEvent,
      false,
      hooks,
    );
  }

  settleParked(
    conversationId: string,
    note = "Superseded by a newer message.",
  ): void {
    const continuations = [...this.continuations.values()].filter(
      (continuation) => continuation.conversationId === conversationId,
    );
    for (const continuation of continuations) {
      if (continuation.blockId) {
        const card = this.cards.get(conversationId)?.get(continuation.blockId);
        if (card?.type === "action_card") {
          this.emitLocalPatch(
            conversationId,
            card.block_id,
            { status: "conflicted", outcome_note: note },
            continuation.emit,
          );
        } else if (card?.type === "approval_card" && card.decision === null) {
          this.emitLocalPatch(
            conversationId,
            card.block_id,
            { decision: "cancelled" },
            continuation.emit,
          );
        }
      }
      this.continuations.delete(
        continuationKey(conversationId, continuation.requestId),
      );
    }
  }

  cancel(conversationId: string): void {
    const script = this.running.get(conversationId);
    if (script) {
      this.cancelScript(script);
      return;
    }
    this.settleParked(conversationId, "This mock action was cancelled.");
  }

  resetConversation(conversationId: string): void {
    this.cancel(conversationId);
    this.running.delete(conversationId);
    this.cards.delete(conversationId);
    for (const [key, continuation] of this.continuations) {
      if (continuation.conversationId === conversationId) {
        this.continuations.delete(key);
      }
    }
  }

  private nextId(kind: string): string {
    this.sequence += 1;
    return `mockchat-${kind}-${String(this.sequence)}`;
  }

  private startSegment(
    conversationId: string,
    steps: readonly ScenarioStep[],
    onEvent: (event: TurnEvent) => void,
    allowConnect: boolean,
    hooks: ScenarioRunHooks,
  ): TurnHandle {
    if (this.running.has(conversationId)) {
      throw new Error("A mock scenario turn is already active.");
    }
    const turnId = this.nextId("turn");
    const plan = this.buildSegmentPlan(turnId, steps, allowConnect);
    const script: RunningScript = {
      conversationId,
      turnId,
      messageId: plan.messageId,
      emit: onEvent,
      hooks,
      timers: new Set(),
      openBlocks: new Map(),
      continuation: plan.continuation,
      messageStarted: false,
      messageCompleted: false,
      cancelled: false,
      finished: false,
    };
    this.running.set(conversationId, script);
    this.schedule(script, plan.items);
    return { turnId, cancel: () => this.cancelScript(script) };
  }

  private buildSegmentPlan(
    turnId: string,
    sourceSteps: readonly ScenarioStep[],
    allowConnect: boolean,
  ): SegmentPlan {
    const items: PlanItem[] = [];
    let elapsed = 0;
    let messageId: string | null = null;
    let blockIndex = 0;
    let lastCard: {
      readonly requestId: string;
      readonly blockId: string;
      readonly kind: "action" | "approval";
      readonly expiresAt: number | null;
    } | null = null;
    let simulatedConnected = new Set(
      Object.keys(this.compiled.flows).filter((slug) =>
        this.world.isConnected(slug),
      ),
    );
    const isConnected = (slug: string) =>
      simulatedConnected.has(slug) || this.world.isConnected(slug);
    const addItem = (item: Omit<PlanItem, "delayMs">, immediate = false) => {
      if (!immediate) elapsed += this.cadenceMs;
      items.push({ ...item, delayMs: elapsed });
    };
    const ensureMessage = () => {
      if (messageId) return messageId;
      messageId = this.nextId("message");
      addItem({
        event: {
          event: "message.started",
          message_id: messageId,
          role: "assistant",
        },
      });
      return messageId;
    };

    addItem(
      { event: { event: "turn.status", turn_id: turnId, status: "running" } },
      true,
    );
    let work = [...sourceSteps];
    let terminal:
      | { readonly status: "completed" }
      | {
          readonly status: "failed";
          readonly error: { readonly code: string; readonly message: string };
        }
      | null = null;
    let continuation: PlannedContinuation | null = null;

    while (work.length > 0) {
      const step = work[0];
      work = work.slice(1);
      if (!step) continue;
      if (step.kind === "run") {
        work = [...(this.compiled.flows[step.flowName]?.steps ?? []), ...work];
        continue;
      }
      if (step.kind === "need") {
        if (!isConnected(step.serviceSlug)) {
          work = [
            ...(this.compiled.flows[step.flowName]?.steps ?? []),
            ...work,
          ];
        }
        continue;
      }
      if (step.kind === "when") {
        if (isConnected(step.serviceSlug) === step.connected) {
          work = [...step.steps, ...work];
        }
        continue;
      }
      if (step.kind === "connect") {
        if (allowConnect) {
          simulatedConnected = new Set([
            ...simulatedConnected,
            step.serviceSlug,
          ]);
          addItem({ effect: () => this.world.connect(step.serviceSlug) });
        }
        continue;
      }
      if (step.kind === "disconnect") {
        simulatedConnected = new Set(
          [...simulatedConnected].filter((slug) => slug !== step.serviceSlug),
        );
        addItem({ effect: () => this.world.disconnect(step.serviceSlug) });
        continue;
      }
      if (step.kind === "wait") {
        elapsed += step.milliseconds;
        continue;
      }
      if (step.kind === "stop") {
        terminal = { status: "completed" };
        break;
      }
      if (step.kind === "fail") {
        terminal = {
          status: "failed",
          error: { code: step.code, message: step.message },
        };
        break;
      }
      if (step.kind === "await") {
        continuation = {
          requestId: lastCard?.requestId ?? turnId,
          blockId: lastCard?.blockId ?? null,
          cardKind: lastCard?.kind ?? "wake",
          branches: step.branches,
          remaining: work,
          expiresAt: lastCard?.expiresAt ?? null,
        };
        break;
      }
      const currentMessageId = ensureMessage();
      if (step.kind === "say") {
        const blockId = this.nextId("block");
        addItem({
          event: {
            event: "block.started",
            message_id: currentMessageId,
            block_id: blockId,
            index: blockIndex,
            block: { type: "text", block_id: blockId, text: "" },
          },
        });
        for (const text of chunkText(step.markdown)) {
          addItem({ event: { event: "block.delta", block_id: blockId, text } });
        }
        addItem({
          event: {
            event: "block.completed",
            block_id: blockId,
            block: { type: "text", block_id: blockId, text: step.markdown },
          },
        });
        blockIndex += 1;
        continue;
      }
      if (step.kind === "tool") {
        const tools = [step];
        while (work[0]?.kind === "tool") {
          const next = work[0];
          if (next?.kind === "tool") tools.push(next);
          work = work.slice(1);
        }
        const blockId = this.nextId("block");
        const baseSteps: RunContentBlock["steps"] = tools.map(
          (tool, index) => ({
            index: index + 1,
            status: index === 0 ? "active" : "waiting",
            label: tool.label,
            meta: index === 0 ? "Running" : "Waiting",
            service_slug: null,
            artifact_id: null,
            approval_request_id: null,
          }),
        );
        addItem({
          event: {
            event: "block.started",
            message_id: currentMessageId,
            block_id: blockId,
            index: blockIndex,
            block: {
              type: "run",
              block_id: blockId,
              title: "RUN",
              steps_total: tools.length,
              steps_complete: 0,
              state: "running",
              steps: baseSteps,
            },
          },
        });
        let settledSteps = [...baseSteps];
        tools.forEach((tool, index) => {
          settledSteps = settledSteps.map((candidate, candidateIndex) => {
            if (candidateIndex === index) {
              return {
                ...candidate,
                status: tool.failed ? ("failed" as const) : ("done" as const),
                meta: tool.result,
              };
            }
            if (candidateIndex === index + 1) {
              return {
                ...candidate,
                status: "active" as const,
                meta: "Running",
              };
            }
            return candidate;
          });
          addItem({
            event: {
              event: "block.updated",
              block_id: blockId,
              patch: {
                steps_complete: index + 1,
                state: tool.failed ? "failed" : "running",
                steps: settledSteps,
              },
            },
          });
        });
        const failed = tools.some((tool) => tool.failed);
        const finalBlock: RunContentBlock = {
          type: "run",
          block_id: blockId,
          title: "RUN",
          steps_total: tools.length,
          steps_complete: tools.length,
          state: failed ? "failed" : "completed",
          steps: settledSteps,
        };
        addItem({
          event: {
            event: "block.completed",
            block_id: blockId,
            block: finalBlock,
          },
        });
        blockIndex += 1;
        continue;
      }
      if (step.kind === "action") {
        const blockId = this.nextId("block");
        const requestId = this.nextId("act");
        const actionRequest = assistantActionRequestSchema.parse({
          schemaVersion: ACTION_SCHEMA_VERSION,
          actorId: "mockchat-engine",
          originTurnId: turnId,
          taskId: this.nextId("task"),
          stepId: this.nextId("step"),
          actionRequestId: requestId,
          action: step.action,
          params: step.target.service
            ? {
                catalogService: {
                  serviceSlug: step.target.service,
                  requestedScopes: step.target.scopes ?? [],
                },
              }
            : {
                customService: {
                  name: step.target.custom?.name,
                  endpointUrl: step.target.custom?.endpointUrl,
                  authMethod: step.target.custom?.authMethod,
                  authKeyName: step.target.custom?.authKeyName,
                },
              },
        });
        const resolved = resolveAssistantAction(actionRequest);
        if (!resolved.supported) {
          throw new ScenarioConfigError(
            `Action ${step.action} is not supported.`,
          );
        }
        const card: ActionCardContentBlock = {
          type: "action_card",
          block_id: blockId,
          action: actionRequest.action,
          action_request_id: requestId,
          origin_turn_id: turnId,
          actor_id: actionRequest.actorId || undefined,
          task_id: actionRequest.taskId,
          step_id: actionRequest.stepId,
          params: resolved.params,
          status: "pending",
          outcome_note: "",
        };
        addItem({
          event: {
            event: "block.started",
            message_id: currentMessageId,
            block_id: blockId,
            index: blockIndex,
            block: card,
          },
        });
        addItem({
          event: { event: "block.completed", block_id: blockId, block: card },
        });
        blockIndex += 1;
        lastCard = { requestId, blockId, kind: "action", expiresAt: null };
        continue;
      }
      if (step.kind === "approval") {
        const blockId = this.nextId("block");
        const requestId = this.nextId("approval");
        const expiresAt = this.now() + APPROVAL_TTL_MS;
        const card: ApprovalCardContentBlock = {
          type: "approval_card",
          block_id: blockId,
          approval_request_id: requestId,
          body: step.body,
          service_slug: step.serviceSlug,
          agent_key_prefix: "mockchat",
          approval_mode: step.options.approvalMode ?? "per_request",
          grant_duration_sec: step.options.grantDurationSec ?? null,
          expires_at: new Date(expiresAt).toISOString(),
          decision: null,
          decision_channel: null,
        };
        addItem({
          event: {
            event: "block.started",
            message_id: currentMessageId,
            block_id: blockId,
            index: blockIndex,
            block: card,
          },
        });
        addItem({
          event: { event: "block.completed", block_id: blockId, block: card },
        });
        blockIndex += 1;
        lastCard = { requestId, blockId, kind: "approval", expiresAt };
        continue;
      }
      if (step.kind === "artifact") {
        const blockId = this.nextId("block");
        const bytes = new TextEncoder().encode(step.preview).byteLength;
        const block: ContentBlock = {
          type: "artifact",
          block_id: blockId,
          artifact_id: this.nextId("artifact"),
          name: step.name,
          mime: step.mime,
          size_bytes: bytes,
          preview: step.preview,
          download_url: `data:${step.mime};charset=utf-8,${encodeURIComponent(step.preview)}`,
        };
        addItem({
          event: {
            event: "block.started",
            message_id: currentMessageId,
            block_id: blockId,
            index: blockIndex,
            block,
          },
        });
        addItem({
          event: { event: "block.completed", block_id: blockId, block },
        });
        blockIndex += 1;
      }
    }

    if (messageId) {
      addItem({ event: { event: "message.completed", message_id: messageId } });
    }
    if (continuation) {
      addItem({
        event: { event: "turn.status", turn_id: turnId, status: "waiting" },
      });
      addItem({
        event: {
          event: "turn.completed",
          turn_id: turnId,
          status: "blocked",
          error: null,
        },
      });
    } else if (terminal?.status === "failed") {
      addItem({
        event: { event: "turn.status", turn_id: turnId, status: "failed" },
      });
      addItem({
        event: {
          event: "turn.completed",
          turn_id: turnId,
          status: "failed",
          error: terminal.error,
        },
      });
    } else {
      addItem({
        event: {
          event: "turn.completed",
          turn_id: turnId,
          status: "completed",
          error: null,
        },
      });
    }
    return { items, messageId, continuation };
  }

  private schedule(script: RunningScript, items: readonly PlanItem[]): void {
    const deliver = (item: PlanItem) => {
      if (script.cancelled || script.finished) return;
      item.effect?.();
      if (!item.event) return;
      const event = {
        ...item.event,
        cursor: this.cursorSource.next(script.conversationId),
      } as TurnEvent;
      this.trackEvent(script, event);
      script.emit(event);
      if (event.event !== "turn.completed") return;
      script.finished = true;
      this.running.delete(script.conversationId);
      if (event.status === "blocked" && script.continuation) {
        const continuation: Continuation = {
          ...script.continuation,
          conversationId: script.conversationId,
          originTurnId: script.turnId,
          emit: script.emit,
        };
        this.continuations.set(
          continuationKey(script.conversationId, continuation.requestId),
          continuation,
        );
        script.hooks.onPark?.(continuation.requestId);
      }
      script.hooks.onTerminal?.(event.status);
    };
    for (const item of items) {
      if (item.delayMs === 0) {
        deliver(item);
        continue;
      }
      const timer = setTimeout(() => {
        script.timers.delete(timer);
        deliver(item);
      }, item.delayMs);
      script.timers.add(timer);
    }
  }

  private trackEvent(script: RunningScript, event: TurnEvent): void {
    if (event.event === "message.started") script.messageStarted = true;
    if (event.event === "message.completed") script.messageCompleted = true;
    if (event.event === "block.started") {
      script.openBlocks.set(event.block_id, event.block);
      this.storeCard(script.conversationId, event.block);
    } else if (event.event === "block.updated") {
      const block = this.cards.get(script.conversationId)?.get(event.block_id);
      if (block)
        this.storeCard(script.conversationId, {
          ...block,
          ...event.patch,
        } as ContentBlock);
    } else if (event.event === "block.completed") {
      script.openBlocks.delete(event.block_id);
      this.storeCard(script.conversationId, event.block);
    }
  }

  private storeCard(conversationId: string, block: ContentBlock): void {
    if (block.type !== "action_card" && block.type !== "approval_card") return;
    const cards = new Map(this.cards.get(conversationId) ?? []);
    cards.set(block.block_id, block);
    this.cards.set(conversationId, cards);
  }

  private cancelScript(script: RunningScript): void {
    if (script.cancelled || script.finished) return;
    script.cancelled = true;
    for (const timer of script.timers) clearTimeout(timer);
    script.timers.clear();
    for (const [blockId, block] of script.openBlocks) {
      this.emitStamped(script.conversationId, script.emit, {
        event: "block.completed",
        block_id: blockId,
        block: toTerminalBlock(block),
      });
    }
    if (script.messageStarted && !script.messageCompleted && script.messageId) {
      this.emitStamped(script.conversationId, script.emit, {
        event: "message.completed",
        message_id: script.messageId,
      });
    }
    this.emitStamped(script.conversationId, script.emit, {
      event: "turn.status",
      turn_id: script.turnId,
      status: "cancelled",
    });
    this.emitStamped(script.conversationId, script.emit, {
      event: "turn.completed",
      turn_id: script.turnId,
      status: "cancelled",
      error: null,
    });
    script.finished = true;
    this.running.delete(script.conversationId);
    script.hooks.onTerminal?.("cancelled");
  }

  private resumeContinuation(
    continuation: Continuation,
    disposition: AwaitDisposition,
    onEvent: (event: TurnEvent) => void,
    allowConnect: boolean,
    hooks: ScenarioRunHooks,
  ): TurnHandle {
    const branch = continuation.branches[disposition] ?? [];
    return this.startSegment(
      continuation.conversationId,
      [...branch, ...continuation.remaining],
      onEvent,
      allowConnect,
      hooks,
    );
  }

  private requireActionCard(
    conversationId: string,
    blockId: string,
  ): ActionCardContentBlock {
    const card = this.cards.get(conversationId)?.get(blockId);
    if (card?.type !== "action_card") {
      throw new Error("Action request was not found.");
    }
    return card;
  }

  private emitLocalPatch(
    conversationId: string,
    blockId: string,
    patch: Extract<TurnEvent, { event: "block.updated" }>["patch"],
    onEvent: (event: TurnEvent) => void,
  ): void {
    const block = this.cards.get(conversationId)?.get(blockId);
    if (block)
      this.storeCard(conversationId, { ...block, ...patch } as ContentBlock);
    this.emitStamped(conversationId, onEvent, {
      event: "block.updated",
      block_id: blockId,
      patch,
    });
  }

  private emitStamped(
    conversationId: string,
    onEvent: (event: TurnEvent) => void,
    event: UnstampedTurnEvent,
  ): void {
    onEvent({
      ...event,
      cursor: this.cursorSource.next(conversationId),
    } as TurnEvent);
  }
}
