import { apiClient } from "@/lib/api-client";
import { AEVATAR_DRAFT_CONVERSATION_PREFIX } from "@/lib/assistant/aevatar-transport";
import {
  AssistantConversationNotFoundError,
  AssistantTurnActiveError,
} from "@/lib/assistant/errors";
import {
  ScenarioEngine,
  compileScenarioSet,
  scenario,
  type CompiledScenarioSet,
  type PreparedActionContinuation,
  type ScenarioMatch,
  type ScenarioRunHooks,
  type WorldPort,
} from "@/lib/assistant/scenario-engine";
import { compiledScenarios } from "@/lib/assistant/scenarios.config";
import { applyTurnEvent, EMPTY_TURN_STATE } from "@/lib/assistant/stream";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";
import { useAuthStore } from "@/stores/auth-store";
import type { ActionReport } from "@/schemas/assistant-actions";
import type { InputAnswer } from "@/schemas/assistant-input";
import type {
  AssistantMessage,
  AssistantTransport,
  Conversation,
  ConversationHistory,
  ProjectionReconcileOutcome,
  TurnEvent,
  TurnHandle,
  TurnReducerState,
} from "@/types/assistant";
import type { KeyInfo } from "@/types/keys";

const MOCK_ID_PREFIX = "mockchat-";
const CLAIMED_FALLBACK_TEXT = "No scenario matched — this is a mock-only chat";
const SUPERSEDED_NOTE = "Superseded by a newer message.";

type OwnershipState =
  | "mock-running"
  | "mock-parked"
  | "verifying"
  | "delegate-active";

interface OverlayGroup {
  readonly anchorMessageId: string | null;
  readonly messageIds: string[];
}

interface ConversationOverlay {
  turnState: TurnReducerState;
  readonly groups: OverlayGroup[];
  activeGroup: OverlayGroup | null;
}

interface VerificationRun {
  readonly controller: AbortController;
  readonly prepared: PreparedActionContinuation;
  activeHandle: TurnHandle | null;
  cancelled: boolean;
}

export interface ScenarioInterceptTransportOptions {
  readonly compiled?: CompiledScenarioSet;
  readonly now?: () => number;
  readonly verifyKey?: (keyId: string, signal: AbortSignal) => Promise<unknown>;
}

export class MockScenariosLoadingError extends Error {
  constructor(state: "idle" | "loading" | "error") {
    super(
      state === "error"
        ? "Mock scenarios could not be loaded. Check the scenario config and retry."
        : "Mock scenarios are still loading. Retry in a moment.",
    );
    this.name = "MockScenariosLoadingError";
  }
}

const fallbackCompiled = compileScenarioSet(
  [
    scenario("claimed-fallback", /[\s\S]*/, (script) =>
      script.say(CLAIMED_FALLBACK_TEXT),
    ),
  ],
  {},
);

function isMockId(value: string): boolean {
  return value.startsWith(MOCK_ID_PREFIX);
}

function isPlaceholderConversation(value: string): boolean {
  return value.startsWith(AEVATAR_DRAFT_CONVERSATION_PREFIX);
}

function maxIso(left: string, right: string): string {
  return left >= right ? left : right;
}

function firstText(messages: readonly AssistantMessage[]): string | null {
  for (const message of messages) {
    if (message.role !== "user") continue;
    const block = message.blocks.find((candidate) => candidate.type === "text");
    if (block?.type === "text") return block.text;
  }
  return null;
}

function userServiceId(report: ActionReport): string | null {
  const resource = report.resource;
  if (!resource || !("userService" in resource)) return null;
  return resource.userService.userServiceId;
}

function verificationSlug(value: unknown): string | null | undefined {
  if (!value || typeof value !== "object") return undefined;
  if (!("catalog_service_slug" in value)) return undefined;
  const slug = (value as { catalog_service_slug?: unknown })
    .catalog_service_slug;
  return typeof slug === "string" || slug === null ? slug : undefined;
}

export class ScenarioInterceptTransport implements AssistantTransport {
  private readonly ownership = new Map<string, OwnershipState>();
  private readonly overlays = new Map<string, ConversationOverlay>();
  private readonly baseHistories = new Map<string, ConversationHistory>();
  private readonly claimedConversations = new Set<string>();
  private readonly tombstones = new Set<string>();
  private readonly verificationRuns = new Map<string, VerificationRun>();
  private readonly engine: ScenarioEngine;
  private readonly fallbackMatch: ScenarioMatch;
  private readonly now: () => number;
  private readonly verifyKey: ScenarioInterceptTransportOptions["verifyKey"];
  private readonly delegate: AssistantTransport;
  private baseList: Conversation[] | null = null;
  private localSequence = 0;

  constructor(
    delegate: AssistantTransport,
    options: ScenarioInterceptTransportOptions = {},
  ) {
    this.delegate = delegate;
    this.now = options.now ?? Date.now;
    this.verifyKey =
      options.verifyKey ??
      ((keyId, signal) =>
        apiClient<KeyInfo>(`/keys/${encodeURIComponent(keyId)}`, { signal }));
    const world: WorldPort = {
      isConnected: (serviceSlug) =>
        useAssistantMockScenariosStore
          .getState()
          .world.connected.includes(serviceSlug),
      connect: (serviceSlug) =>
        useAssistantMockScenariosStore.getState().connectService(serviceSlug),
      disconnect: (serviceSlug) =>
        useAssistantMockScenariosStore
          .getState()
          .disconnectService(serviceSlug),
    };
    const cursorSource = {
      next: (conversationId: string) =>
        this.ensureOverlay(conversationId).turnState.lastCursor + 1,
    };
    this.engine = new ScenarioEngine(
      options.compiled ?? compiledScenarios,
      world,
      cursorSource,
      { now: this.now },
    );
    const fallbackMatcher = new ScenarioEngine(
      fallbackCompiled,
      world,
      cursorSource,
      { now: this.now },
    );
    const fallbackMatch = fallbackMatcher.match("");
    if (!fallbackMatch)
      throw new Error("Mock-only fallback scenario is invalid.");
    this.fallbackMatch = fallbackMatch;
  }

  async listConversations(): Promise<Conversation[]> {
    if (!this.hasMockActivity()) {
      const delegated = await this.delegate.listConversations();
      this.baseList = delegated.filter(
        (conversation) => !this.tombstones.has(conversation.id),
      );
    }
    const projected = new Map<string, Conversation>();
    for (const conversation of this.baseList ?? []) {
      if (!this.tombstones.has(conversation.id)) {
        projected.set(conversation.id, conversation);
      }
    }
    for (const conversationId of this.overlays.keys()) {
      if (this.tombstones.has(conversationId)) continue;
      const base =
        projected.get(conversationId) ??
        this.baseHistories.get(conversationId)?.conversation;
      if (!base) continue;
      projected.set(
        conversationId,
        this.projectConversation(conversationId, base),
      );
    }
    return [...projected.values()].sort((left, right) =>
      right.last_message_at.localeCompare(left.last_message_at),
    );
  }

  async createConversation(): Promise<Conversation> {
    const conversation = await this.delegate.createConversation();
    if (!this.tombstones.has(conversation.id)) {
      this.baseHistories.set(conversation.id, {
        conversation,
        messages: [],
        has_more: false,
      });
      if (this.baseList) this.baseList = [conversation, ...this.baseList];
    }
    return conversation;
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    this.requireNotDeleted(conversationId);
    if (!this.claimedConversations.has(conversationId)) {
      const state = this.ownership.get(conversationId);
      if (state !== "mock-running" && state !== "verifying") {
        const delegated = await this.delegate.getHistory(conversationId);
        if (!this.tombstones.has(conversationId)) {
          this.baseHistories.set(conversationId, delegated);
        }
      }
    }
    const base = this.baseHistories.get(conversationId);
    if (!base) throw new AssistantConversationNotFoundError();
    return this.projectHistory(conversationId, base);
  }

  /**
   * Mock-owned conversations exist only in the wrapper, so there is no
   * server-side projection to wait on -- they are materialized by
   * definition. Anything the wrapper does not own is reconciled by the
   * real transport. Ownership matches `getHistory`'s read-path rule so a
   * claimed or mock-running conversation never reaches the backend.
   */
  async reconcileProjection(
    conversationId: string,
  ): Promise<ProjectionReconcileOutcome> {
    if (!this.mockOwnsProjection(conversationId)) {
      return this.delegate.reconcileProjection(conversationId);
    }
    return { status: "materialized", conversationId };
  }

  releaseProjectionWaiter(conversationId: string): void {
    if (!this.mockOwnsProjection(conversationId)) {
      this.delegate.releaseProjectionWaiter(conversationId);
    }
  }

  private mockOwnsProjection(conversationId: string): boolean {
    if (this.claimedConversations.has(conversationId)) return true;
    const state = this.ownership.get(conversationId);
    return state === "mock-running" || state === "verifying";
  }

  async deleteConversation(conversationId: string): Promise<void> {
    if (this.tombstones.has(conversationId)) return;
    const wrapperOwned =
      this.overlays.has(conversationId) ||
      this.claimedConversations.has(conversationId) ||
      this.ownership.has(conversationId) ||
      this.verificationRuns.has(conversationId);
    if (!wrapperOwned) {
      await this.delegate.deleteConversation(conversationId);
      return;
    }
    const verification = this.verificationRuns.get(conversationId);
    if (verification) {
      verification.cancelled = true;
      verification.controller.abort();
      verification.prepared.cancel();
      verification.activeHandle?.cancel();
      this.verificationRuns.delete(conversationId);
    }
    this.engine.resetConversation(conversationId);
    this.ownership.delete(conversationId);
    this.overlays.delete(conversationId);
    this.baseHistories.delete(conversationId);
    this.claimedConversations.delete(conversationId);
    this.tombstones.add(conversationId);
    if (this.baseList) {
      this.baseList = this.baseList.filter(
        (conversation) => conversation.id !== conversationId,
      );
    }
    await this.delegate.deleteConversation(conversationId);
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    this.requireNotDeleted(conversationId);
    const state = this.ownership.get(conversationId);
    if (
      state === "mock-running" ||
      state === "verifying" ||
      state === "delegate-active"
    ) {
      throw new AssistantTurnActiveError();
    }
    if (state === "mock-parked") {
      this.engine.settleParked(conversationId, SUPERSEDED_NOTE);
      this.ownership.delete(conversationId);
    }

    const store = useAssistantMockScenariosStore.getState();
    let match: ScenarioMatch | null = null;
    if (store.enabled) {
      if (store.engineState !== "ready") {
        throw new MockScenariosLoadingError(store.engineState);
      }
      match = this.engine.match(content, store.disabledScenarioIds);
      store.noteActivity({
        scenarioId: match?.scenarioId ?? null,
        matched: match !== null,
        at: this.now(),
      });
    }

    const claimed = this.claimedConversations.has(conversationId);
    if (!match && !claimed) {
      this.ownership.set(conversationId, "delegate-active");
      try {
        return this.delegate.sendMessage(conversationId, content, (event) => {
          onEvent(event);
          if (event.event === "turn.completed") {
            this.ownership.delete(conversationId);
          }
        });
      } catch (error) {
        this.ownership.delete(conversationId);
        throw error;
      }
    }

    if (!match) match = this.fallbackMatch;
    if (isPlaceholderConversation(conversationId)) {
      this.claimedConversations.add(conversationId);
    }
    this.beginMockGroup(conversationId, content);
    this.ownership.set(conversationId, "mock-running");
    return this.engine.play(
      conversationId,
      match,
      this.eventSink(conversationId, onEvent),
      this.runHooks(conversationId),
    );
  }

  cancelActiveTurn(conversationId: string): void {
    if (this.tombstones.has(conversationId)) return;
    const state = this.ownership.get(conversationId);
    if (state === "verifying") {
      this.cancelVerification(conversationId);
      return;
    }
    if (state === "mock-running" || state === "mock-parked") {
      this.engine.cancel(conversationId);
      if (state === "mock-parked") this.ownership.delete(conversationId);
      return;
    }
    this.delegate.cancelActiveTurn(conversationId);
  }

  stopTask(conversationId: string): Promise<void> {
    return this.delegate.stopTask(conversationId);
  }

  steerTask(conversationId: string, instruction: string): Promise<void> {
    return this.delegate.steerTask(conversationId, instruction);
  }

  retryStep(conversationId: string, stepId: string): Promise<void> {
    return this.delegate.retryStep(conversationId, stepId);
  }

  skipStep(conversationId: string, stepId: string): Promise<void> {
    return this.delegate.skipStep(conversationId, stepId);
  }

  resolvePlan(
    conversationId: string,
    blockId: string,
    confirmed: boolean,
  ): Promise<void> {
    return this.delegate.resolvePlan(conversationId, blockId, confirmed);
  }

  async decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): Promise<TurnHandle | null> {
    if (!isMockId(blockId)) {
      return this.delegate.decideApproval(
        conversationId,
        blockId,
        approved,
        onEvent,
      );
    }
    const priorOwnership = this.ownership.get(conversationId);
    this.requireMockMutationAllowed(conversationId);
    this.beginContinuationGroup(conversationId);
    this.ownership.set(conversationId, "mock-running");
    try {
      const handle = this.engine.decideApproval(
        conversationId,
        blockId,
        approved,
        this.eventSink(conversationId, onEvent),
        this.runHooks(conversationId),
      );
      if (!handle) {
        this.discardEmptyActiveGroup(conversationId);
        this.ownership.delete(conversationId);
      }
      return handle;
    } catch (error) {
      this.discardEmptyActiveGroup(conversationId);
      this.restoreOwnership(conversationId, priorOwnership);
      throw error;
    }
  }

  resolveInput(
    conversationId: string,
    blockId: string,
    answer: InputAnswer,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): Promise<TurnHandle | null> {
    return this.delegate.resolveInput(conversationId, blockId, answer, onEvent);
  }

  setActionCardInProgress(
    conversationId: string,
    blockId: string,
    inProgress: boolean,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): void {
    if (!isMockId(blockId)) {
      this.delegate.setActionCardInProgress(
        conversationId,
        blockId,
        inProgress,
        onEvent,
      );
      return;
    }
    if (this.tombstones.has(conversationId)) return;
    this.engine.setActionCardInProgress(
      conversationId,
      blockId,
      inProgress,
      this.eventSink(conversationId, onEvent),
    );
  }

  blockActionCard(
    conversationId: string,
    blockId: string,
    note: string,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): void {
    if (!isMockId(blockId)) {
      this.delegate.blockActionCard(conversationId, blockId, note, onEvent);
      return;
    }
    if (this.tombstones.has(conversationId)) return;
    this.engine.blockActionCard(
      conversationId,
      blockId,
      note,
      this.eventSink(conversationId, onEvent),
    );
  }

  continueActions(
    conversationId: string,
    originTurnId: string,
    reports: readonly ActionReport[],
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): TurnHandle | null {
    if (!isMockId(originTurnId)) {
      return this.delegate.continueActions(
        conversationId,
        originTurnId,
        reports,
        onEvent,
      );
    }
    const priorOwnership = this.ownership.get(conversationId);
    this.requireMockMutationAllowed(conversationId);
    this.ownership.set(conversationId, "verifying");
    let prepared: PreparedActionContinuation;
    try {
      prepared = this.engine.prepareActionContinuation(
        conversationId,
        originTurnId,
        reports,
        this.eventSink(conversationId, onEvent),
      );
    } catch (error) {
      this.restoreOwnership(conversationId, priorOwnership);
      throw error;
    }
    if (prepared.report.disposition !== "completed") {
      this.beginContinuationGroup(conversationId);
      this.ownership.set(conversationId, "mock-running");
      return prepared.resume("unverified", this.runHooks(conversationId));
    }
    return this.startVerification(conversationId, prepared, onEvent);
  }

  wakeActions(
    conversationId: string,
    originTurnId: string,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): TurnHandle {
    if (!isMockId(originTurnId)) {
      return this.delegate.wakeActions(conversationId, originTurnId, onEvent);
    }
    const priorOwnership = this.ownership.get(conversationId);
    this.requireMockMutationAllowed(conversationId);
    this.beginContinuationGroup(conversationId);
    this.ownership.set(conversationId, "mock-running");
    try {
      return this.engine.wake(
        conversationId,
        originTurnId,
        this.eventSink(conversationId, onEvent),
        this.runHooks(conversationId),
      );
    } catch (error) {
      this.discardEmptyActiveGroup(conversationId);
      this.restoreOwnership(conversationId, priorOwnership);
      throw error;
    }
  }

  private startVerification(
    conversationId: string,
    prepared: PreparedActionContinuation,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    const controller = new AbortController();
    const verification: VerificationRun = {
      controller,
      prepared,
      activeHandle: null,
      cancelled: false,
    };
    this.verificationRuns.set(conversationId, verification);
    const provisional: TurnHandle = {
      get turnId() {
        return verification.activeHandle?.turnId ?? null;
      },
      cancel: () => this.cancelVerification(conversationId),
    };
    void this.resolveVerification(conversationId, verification, onEvent);
    return provisional;
  }

  private async resolveVerification(
    conversationId: string,
    verification: VerificationRun,
    onEvent: (event: TurnEvent) => void,
  ): Promise<void> {
    let result: "verified" | "mismatch" | "unverified" = "unverified";
    const keyId = userServiceId(verification.prepared.report);
    const expected = verification.prepared.expectedServiceSlug;
    if (keyId && expected) {
      try {
        const key = await this.verifyKey?.(
          keyId,
          verification.controller.signal,
        );
        const actual = verificationSlug(key);
        if (actual !== undefined) {
          result = actual === expected ? "verified" : "mismatch";
        }
      } catch {
        result = "unverified";
      }
    }
    if (
      verification.cancelled ||
      verification.controller.signal.aborted ||
      this.tombstones.has(conversationId) ||
      this.verificationRuns.get(conversationId) !== verification
    ) {
      return;
    }
    this.verificationRuns.delete(conversationId);
    this.beginContinuationGroup(conversationId);
    this.ownership.set(conversationId, "mock-running");
    verification.activeHandle = verification.prepared.resume(
      result,
      this.runHooks(conversationId),
    );
    if (!verification.activeHandle) {
      this.discardEmptyActiveGroup(conversationId);
      this.ownership.delete(conversationId);
    }
    void onEvent;
  }

  private cancelVerification(conversationId: string): void {
    const verification = this.verificationRuns.get(conversationId);
    if (!verification) {
      this.engine.cancel(conversationId);
      return;
    }
    verification.cancelled = true;
    verification.controller.abort();
    this.engine.settleParked(
      conversationId,
      "Connection verification was cancelled.",
    );
    verification.prepared.cancel();
    verification.activeHandle?.cancel();
    this.verificationRuns.delete(conversationId);
    this.ownership.delete(conversationId);
  }

  private requireMockMutationAllowed(conversationId: string): void {
    this.requireNotDeleted(conversationId);
    const state = this.ownership.get(conversationId);
    if (
      state === "mock-running" ||
      state === "verifying" ||
      state === "delegate-active"
    ) {
      throw new AssistantTurnActiveError();
    }
  }

  private restoreOwnership(
    conversationId: string,
    ownership: OwnershipState | undefined,
  ): void {
    if (ownership) {
      this.ownership.set(conversationId, ownership);
    } else {
      this.ownership.delete(conversationId);
    }
  }

  private requireNotDeleted(conversationId: string): void {
    if (this.tombstones.has(conversationId)) {
      throw new AssistantConversationNotFoundError();
    }
  }

  private runHooks(conversationId: string): ScenarioRunHooks {
    return {
      onPark: () => this.ownership.set(conversationId, "mock-parked"),
      onTerminal: (status) => {
        if (status !== "blocked") this.ownership.delete(conversationId);
      },
    };
  }

  private eventSink(
    conversationId: string,
    onEvent: (event: TurnEvent) => void,
  ): (event: TurnEvent) => void {
    return (event) => {
      if (this.tombstones.has(conversationId)) return;
      const overlay = this.ensureOverlay(conversationId);
      overlay.turnState = applyTurnEvent(
        overlay.turnState,
        event,
        new Date(this.now()).toISOString(),
      );
      if (event.event === "message.started" && overlay.activeGroup) {
        overlay.activeGroup.messageIds.push(event.message_id);
      }
      if (event.event === "turn.completed") overlay.activeGroup = null;
      onEvent(event);
    };
  }

  private ensureOverlay(conversationId: string): ConversationOverlay {
    let overlay = this.overlays.get(conversationId);
    if (!overlay) {
      overlay = {
        turnState: EMPTY_TURN_STATE,
        groups: [],
        activeGroup: null,
      };
      this.overlays.set(conversationId, overlay);
    }
    return overlay;
  }

  private beginMockGroup(conversationId: string, content: string): void {
    const overlay = this.ensureOverlay(conversationId);
    const base = this.baseHistories.get(conversationId);
    const group: OverlayGroup = {
      anchorMessageId: base?.messages.at(-1)?.id ?? null,
      messageIds: [],
    };
    const createdAt = new Date(this.now()).toISOString();
    this.localSequence += 1;
    const messageId = `${MOCK_ID_PREFIX}user-message-${String(this.localSequence)}`;
    this.localSequence += 1;
    const blockId = `${MOCK_ID_PREFIX}user-block-${String(this.localSequence)}`;
    const message: AssistantMessage = {
      id: messageId,
      role: "user",
      schema_version: 1,
      blocks: [{ type: "text", block_id: blockId, text: content.trim() }],
      created_at: createdAt,
    };
    group.messageIds.push(messageId);
    overlay.groups.push(group);
    overlay.activeGroup = group;
    overlay.turnState = {
      ...overlay.turnState,
      messages: [...overlay.turnState.messages, message],
    };
  }

  private beginContinuationGroup(conversationId: string): void {
    const overlay = this.ensureOverlay(conversationId);
    const base = this.baseHistories.get(conversationId);
    const group: OverlayGroup = {
      anchorMessageId: base?.messages.at(-1)?.id ?? null,
      messageIds: [],
    };
    overlay.groups.push(group);
    overlay.activeGroup = group;
  }

  private discardEmptyActiveGroup(conversationId: string): void {
    const overlay = this.overlays.get(conversationId);
    const group = overlay?.activeGroup;
    if (!overlay || !group || group.messageIds.length > 0) return;
    const index = overlay.groups.indexOf(group);
    if (index >= 0) overlay.groups.splice(index, 1);
    overlay.activeGroup = null;
  }

  private projectHistory(
    conversationId: string,
    base: ConversationHistory,
  ): ConversationHistory {
    const overlay = this.overlays.get(conversationId);
    if (!overlay) return base;
    const overlayMessages = new Map(
      overlay.turnState.messages.map((message) => [message.id, message]),
    );
    const groupsByAnchor = new Map<string, OverlayGroup[]>();
    const tailGroups: OverlayGroup[] = [];
    const baseIds = new Set(base.messages.map((message) => message.id));
    for (const group of overlay.groups) {
      if (group.anchorMessageId && baseIds.has(group.anchorMessageId)) {
        const groups = groupsByAnchor.get(group.anchorMessageId) ?? [];
        groups.push(group);
        groupsByAnchor.set(group.anchorMessageId, groups);
      } else {
        tailGroups.push(group);
      }
    }
    const messages: AssistantMessage[] = [];
    const appendGroup = (group: OverlayGroup) => {
      for (const messageId of group.messageIds) {
        const message = overlayMessages.get(messageId);
        if (message) messages.push(message);
      }
    };
    for (const message of base.messages) {
      messages.push(message);
      for (const group of groupsByAnchor.get(message.id) ?? [])
        appendGroup(group);
    }
    for (const group of tailGroups) appendGroup(group);
    return {
      ...base,
      conversation: this.projectConversation(conversationId, base.conversation),
      messages,
    };
  }

  private projectConversation(
    conversationId: string,
    base: Conversation,
  ): Conversation {
    const overlay = this.overlays.get(conversationId);
    if (!overlay || overlay.turnState.messages.length === 0) return base;
    const newest = overlay.turnState.messages.reduce(
      (latest, message) => maxIso(latest, message.created_at),
      base.last_message_at,
    );
    const overlayCount = overlay.turnState.messages.length;
    const claimed = this.claimedConversations.has(conversationId);
    const title = claimed
      ? (firstText(overlay.turnState.messages)?.slice(0, 40) ?? base.title)
      : base.title;
    return {
      ...base,
      title,
      last_message_at: newest,
      ...(typeof base.message_count === "number"
        ? { message_count: base.message_count + overlayCount }
        : claimed
          ? { message_count: overlayCount }
          : {}),
    };
  }

  private hasMockActivity(): boolean {
    return [...this.ownership.values()].some(
      (state) => state === "mock-running" || state === "verifying",
    );
  }
}

export function createScenarioInterceptTransport(
  delegate: AssistantTransport,
): ScenarioInterceptTransport {
  const transport = new ScenarioInterceptTransport(delegate);
  useAssistantMockScenariosStore.getState().setEngineState("ready");
  return transport;
}

export interface ScenarioInterceptorShell {
  current(): AssistantTransport;
  install(transport: AssistantTransport): boolean;
}

const installedShells = new WeakSet<object>();
let authSubscriptionInstalled = false;

function ensureCurrentMockScenarioUser(): void {
  const userId = useAuthStore.getState().user?.id;
  if (userId) {
    useAssistantMockScenariosStore.getState().ensureUser(userId);
  }
}

function installAuthSubscription(): void {
  if (authSubscriptionInstalled) return;
  authSubscriptionInstalled = true;
  ensureCurrentMockScenarioUser();
  useAuthStore.subscribe((state, previous) => {
    const userId = state.user?.id;
    if (userId && userId !== previous.user?.id) {
      useAssistantMockScenariosStore.getState().ensureUser(userId);
    }
  });
}

export function installScenarioInterceptor(
  shell: ScenarioInterceptorShell,
): void {
  if (installedShells.has(shell)) return;
  installedShells.add(shell);
  shell.install(createScenarioInterceptTransport(shell.current()));
  installAuthSubscription();
}
