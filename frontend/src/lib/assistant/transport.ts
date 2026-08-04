import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import { ApiError } from "@/lib/api-client";
import { composeUnreportedCompletedNote } from "@/lib/assistant/action-notes";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import {
  assistantMockStore,
  createScriptedTurn,
  resetAssistantMockStore,
} from "@/lib/assistant/mock-data";
import { toTerminalBlock } from "@/lib/assistant/stream";
import {
  actionReportSchema,
  buildActionContinueBody,
  buildActionWakeBody,
  type ActionReport,
} from "@/schemas/assistant-actions";
import type {
  AssistantTransport,
  Conversation,
  ConversationHistory,
  ProjectionReconcileOutcome,
  TurnEvent,
  TurnHandle,
} from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";

export { AssistantTurnActiveError };

const EVENT_CADENCE_MS = 100;

/**
 * Dev/e2e-only fault injection for the mock transport. The e2e harness
 * (frontend/e2e/) sets `window.__assistantMockFaults` via addInitScript to
 * reproduce latency- and failure-dependent states the instant mock cannot
 * otherwise reach (slow transcript reads, transcript errors, a stream that
 * never starts). Inert in production: the mock transport itself is only
 * selected in dev/test sessions.
 */
export interface AssistantMockFaults {
  /** Delay every getHistory resolution by this many ms. */
  readonly historyDelayMs?: number;
  /** Make every getHistory reject with an ApiError of this HTTP status. */
  readonly historyErrorStatus?: number;
  /** Accept sends (user message lands) but never emit a single turn event. */
  readonly sendSilent?: boolean;
  /** Rename a locally pending draft to its canonical id on the first send. */
  readonly aliasOnFirstSend?: boolean;
}

function mockFaults(): AssistantMockFaults {
  return (
    (globalThis as { __assistantMockFaults?: AssistantMockFaults })
      .__assistantMockFaults ?? {}
  );
}

interface RunningScript {
  readonly turnId: string;
  readonly messageId: string;
  readonly onEvent: (event: TurnEvent) => void;
  readonly timers: Set<ReturnType<typeof setTimeout>>;
  readonly openBlockIds: Set<string>;
  cancelled: boolean;
  finished: boolean;
}

interface PendingMockActionBatch {
  readonly originTurnId: string;
  readonly reports: readonly ActionReport[];
  readonly onEvent: (event: TurnEvent) => void;
}

class MockAssistantTransport implements AssistantTransport {
  private readonly running = new Map<string, RunningScript>();
  private readonly pendingActions = new Map<string, PendingMockActionBatch[]>();

  async listConversations(): Promise<Conversation[]> {
    return assistantMockStore.listConversations();
  }

  async createConversation(): Promise<Conversation> {
    return assistantMockStore.createConversation({
      pendingAlias: mockFaults().aliasOnFirstSend,
    });
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    const faults = mockFaults();
    if (faults.historyDelayMs) {
      await new Promise((resolve) =>
        setTimeout(resolve, faults.historyDelayMs),
      );
    }
    if (faults.historyErrorStatus) {
      throw new ApiError(faults.historyErrorStatus, {
        error: "mock_history_fault",
        error_code: 0,
        message: "Injected mock transcript failure.",
      });
    }
    return assistantMockStore.getHistory(conversationId);
  }

  async reconcileProjection(conversationId: string) {
    return { status: "materialized" as const, conversationId };
  }

  releaseProjectionWaiter(conversationId: string): void {
    void conversationId;
  }

  async deleteConversation(conversationId: string): Promise<void> {
    for (const address of assistantMockStore.conversationAddresses(
      conversationId,
    )) {
      const script = this.running.get(address);
      if (script) this.cancelScript(address, script);
      this.pendingActions.delete(address);
    }
    assistantMockStore.deleteConversation(conversationId);
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    const currentStatus =
      assistantMockStore.getTurnState(conversationId).activeTurn?.status;
    if (this.running.has(conversationId) || isTurnActive(currentStatus)) {
      throw new AssistantTurnActiveError();
    }

    assistantMockStore.appendUserMessage(conversationId, content);
    if (
      mockFaults().aliasOnFirstSend &&
      conversationId.startsWith("local-pending-")
    ) {
      assistantMockStore.aliasConversation(conversationId);
    }
    const turnId = assistantMockStore.nextId("turn");
    const messageId = assistantMockStore.nextId("assistant-message");
    const blockId = assistantMockStore.nextId("assistant-block");
    const events = createScriptedTurn(turnId, messageId, blockId);
    return this.startScript(
      conversationId,
      turnId,
      messageId,
      events,
      onEvent,
      {
        silent: mockFaults().sendSilent,
      },
    );
  }

  cancelActiveTurn(conversationId: string): void {
    for (const address of assistantMockStore.conversationAddresses(
      conversationId,
    )) {
      const script = this.running.get(address);
      if (script) {
        this.cancelScript(address, script);
        return;
      }
    }
  }

  async decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
  ): Promise<TurnHandle | null> {
    assistantMockStore.decideApproval(conversationId, blockId, approved);
    // The scripted store settles the card synchronously; there is no
    // continuation stream to hand back.
    return null;
  }

  setActionCardInProgress(
    conversationId: string,
    blockId: string,
    inProgress: boolean,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): void {
    const block = assistantMockStore.findBlock(conversationId, blockId);
    if (block?.type !== "action_card") {
      throw new Error("Action request was not found.");
    }
    if (
      block.status === "blocked" ||
      block.status === "completed" ||
      block.status === "conflicted" ||
      block.status === "declined" ||
      block.status === "failed" ||
      block.status === "unsupported"
    ) {
      return;
    }
    this.emitLocalActionPatch(
      conversationId,
      blockId,
      {
        status: inProgress ? "in_progress" : "pending",
        outcome_note: "",
      },
      onEvent,
    );
  }

  blockActionCard(
    conversationId: string,
    blockId: string,
    note: string,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): void {
    const block = assistantMockStore.findBlock(conversationId, blockId);
    if (block?.type !== "action_card") {
      throw new Error("Action request was not found.");
    }
    if (
      block.status === "completed" ||
      block.status === "conflicted" ||
      block.status === "declined" ||
      block.status === "failed"
    ) {
      return;
    }
    this.emitLocalActionPatch(
      conversationId,
      blockId,
      { status: "blocked", outcome_note: note },
      onEvent,
    );
  }

  continueActions(
    conversationId: string,
    originTurnId: string,
    reports: readonly ActionReport[],
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): TurnHandle | null {
    const validated = reports.map((report) => actionReportSchema.parse(report));
    const actionLookup = new Map<string, string>();
    for (const report of validated) {
      const messages = assistantMockStore.getHistory(conversationId).messages;
      const card = messages
        .flatMap((message) => message.blocks)
        .find(
          (block) =>
            block.type === "action_card" &&
            block.action_request_id === report.actionRequestId,
        );
      if (card?.type === "action_card") {
        actionLookup.set(report.actionRequestId, card.action);
      }
      const refusedByCardState =
        card?.type === "action_card" &&
        (card.status === "conflicted" ||
          (card.status === "blocked" && report.disposition === "completed"));
      if (refusedByCardState) {
        if (
          card?.type === "action_card" &&
          report.disposition === "completed" &&
          report.resource
        ) {
          this.emitLocalActionPatch(
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
    }
    buildActionContinueBody(
      conversationId,
      crypto.randomUUID(),
      originTurnId,
      validated,
      actionLookup,
    );
    for (const report of validated) {
      const messages = assistantMockStore.getHistory(conversationId).messages;
      const card = messages
        .flatMap((message) => message.blocks)
        .find(
          (block) =>
            block.type === "action_card" &&
            block.action_request_id === report.actionRequestId,
        );
      if (card?.type !== "action_card") continue;
      const status =
        report.disposition === "completed"
          ? "completed"
          : report.disposition === "declined"
            ? "declined"
            : "failed";
      const outcomeNote =
        report.disposition === "completed"
          ? "Reported — awaiting assistant verification."
          : report.disposition === "declined"
            ? "You declined this request. No service was connected and no credential was shared."
            : "The connection could not be completed. Ask the assistant to request it again.";
      this.emitLocalActionPatch(
        conversationId,
        card.block_id,
        { status, outcome_note: outcomeNote },
        onEvent,
      );
    }
    const pending = this.pendingActions.get(conversationId) ?? [];
    pending.push({ originTurnId, reports: validated, onEvent });
    this.pendingActions.set(conversationId, pending);
    return this.startPendingAction(conversationId);
  }

  wakeActions(
    conversationId: string,
    originTurnId: string,
    onEvent: (event: TurnEvent) => void = () => undefined,
  ): TurnHandle {
    buildActionWakeBody(crypto.randomUUID(), originTurnId);
    const currentStatus =
      assistantMockStore.getTurnState(conversationId).activeTurn?.status;
    if (this.running.has(conversationId) || isTurnActive(currentStatus)) {
      throw new AssistantTurnActiveError();
    }
    const pending = this.pendingActions.get(conversationId) ?? [];
    pending.push({ originTurnId, reports: [], onEvent });
    this.pendingActions.set(conversationId, pending);
    const handle = this.startPendingAction(conversationId);
    if (!handle) throw new AssistantTurnActiveError();
    return handle;
  }

  reset(now: () => number = Date.now): void {
    for (const script of this.running.values()) {
      script.cancelled = true;
      for (const timer of script.timers) clearTimeout(timer);
    }
    this.running.clear();
    this.pendingActions.clear();
    resetAssistantMockStore(now);
  }

  private emitLocalActionPatch(
    conversationId: string,
    blockId: string,
    patch: Extract<TurnEvent, { event: "block.updated" }>["patch"],
    onEvent: (event: TurnEvent) => void,
  ): void {
    const event: TurnEvent = {
      cursor: assistantMockStore.getTurnState(conversationId).lastCursor + 1,
      event: "block.updated",
      block_id: blockId,
      patch,
    };
    assistantMockStore.applyEvent(conversationId, event);
    onEvent(event);
  }

  private startPendingAction(conversationId: string): TurnHandle | null {
    if (this.running.has(conversationId)) return null;
    const pending = this.pendingActions.get(conversationId);
    const batch = pending?.shift();
    if (!batch) return null;
    if (pending?.length === 0) this.pendingActions.delete(conversationId);

    const turnId = assistantMockStore.nextId("turn-action");
    const messageId = assistantMockStore.nextId("assistant-action-message");
    const blockId = assistantMockStore.nextId("assistant-action-block");
    const wake = batch.reports.length === 0;
    const declined =
      !wake &&
      batch.reports.every((report) => report.disposition === "declined");
    const text = wake
      ? "The assistant resumed the blocked turn."
      : declined
        ? "Understood. I did not connect the service, and I will continue without it."
        : "The service connection is ready. I can now continue with brokered access.";
    const events: TurnEvent[] = [
      { cursor: 1, event: "turn.status", turn_id: turnId, status: "running" },
      {
        cursor: 2,
        event: "message.started",
        message_id: messageId,
        role: "assistant",
      },
      {
        cursor: 3,
        event: "block.started",
        message_id: messageId,
        block_id: blockId,
        index: 0,
        block: { type: "text", block_id: blockId, text },
      },
      {
        cursor: 4,
        event: "block.completed",
        block_id: blockId,
        block: { type: "text", block_id: blockId, text },
      },
      { cursor: 5, event: "message.completed", message_id: messageId },
      {
        cursor: 6,
        event: "turn.completed",
        turn_id: turnId,
        status: "completed",
        error: null,
      },
    ];
    return this.startScript(
      conversationId,
      turnId,
      messageId,
      events,
      batch.onEvent,
    );
  }

  private startScript(
    conversationId: string,
    turnId: string,
    messageId: string,
    events: readonly TurnEvent[],
    onEvent: (event: TurnEvent) => void,
    options: { readonly silent?: boolean } = {},
  ): TurnHandle {
    const script: RunningScript = {
      turnId,
      messageId,
      onEvent,
      timers: new Set(),
      openBlockIds: new Set(),
      cancelled: false,
      finished: false,
    };
    this.running.set(conversationId, script);

    // A "silent" send mirrors the real transport hanging before response
    // headers: the user message landed, the turn is reserved, but no event
    // will ever arrive. Cancel still settles it.
    if (options.silent) {
      return {
        turnId,
        cancel: () => this.cancelScript(conversationId, script),
      };
    }

    const emit = (sourceEvent: TurnEvent) => {
      if (script.cancelled || script.finished) return;
      const event = {
        ...sourceEvent,
        cursor: assistantMockStore.getTurnState(conversationId).lastCursor + 1,
      } as TurnEvent;
      assistantMockStore.applyEvent(conversationId, event);
      this.trackLifecycle(script, event);
      onEvent(event);
      if (event.event === "turn.completed") {
        script.finished = true;
        this.running.delete(conversationId);
        queueMicrotask(() => this.startPendingAction(conversationId));
      }
    };

    const first = events[0];
    if (first) emit(first);
    events.slice(1).forEach((event, index) => {
      const timer = setTimeout(
        () => {
          script.timers.delete(timer);
          emit(event);
        },
        (index + 1) * EVENT_CADENCE_MS,
      );
      script.timers.add(timer);
    });
    return {
      turnId,
      cancel: () => this.cancelScript(conversationId, script),
    };
  }

  private trackLifecycle(script: RunningScript, event: TurnEvent): void {
    if (event.event === "block.started") {
      script.openBlockIds.add(event.block_id);
    } else if (event.event === "block.completed") {
      script.openBlockIds.delete(event.block_id);
    }
  }

  private cancelScript(conversationId: string, script: RunningScript): void {
    if (script.cancelled || script.finished) return;
    script.cancelled = true;
    for (const timer of script.timers) clearTimeout(timer);
    script.timers.clear();

    let cursor = assistantMockStore.getTurnState(conversationId).lastCursor;
    const emitCancellation = (event: TurnEvent) => {
      assistantMockStore.applyEvent(conversationId, event);
      script.onEvent(event);
    };

    for (const blockId of script.openBlockIds) {
      const block = assistantMockStore.findBlock(conversationId, blockId);
      if (!block) continue;
      cursor += 1;
      emitCancellation({
        cursor,
        event: "block.completed",
        block_id: blockId,
        block: toTerminalBlock(block),
      });
    }
    if (cursor >= 2) {
      cursor += 1;
      emitCancellation({
        cursor,
        event: "message.completed",
        message_id: script.messageId,
      });
    }
    cursor += 1;
    emitCancellation({
      cursor,
      event: "turn.status",
      turn_id: script.turnId,
      status: "cancelled",
    });
    cursor += 1;
    emitCancellation({
      cursor,
      event: "turn.completed",
      turn_id: script.turnId,
      status: "cancelled",
      error: null,
    });
    script.finished = true;
    this.running.delete(conversationId);
    queueMicrotask(() => this.startPendingAction(conversationId));
  }
}

/**
 * Which transport a session gets. Vitest and dev `?mock` demo sessions stay
 * on the scripted transport; every other session — production above all —
 * talks to Aevatar's nyxid-chat API through the NyxID proxy. The `?mock`
 * switch mirrors the page-level mock layer (lib/mock-data.ts `isMockMode`),
 * duplicated as a cheap check so the prod bundle does not statically pull
 * that module in; outside dev builds it is inert.
 */
export function selectAssistantTransportKind(env: {
  readonly mode: string;
  readonly dev: boolean;
  readonly search: string;
}): "mock" | "aevatar" {
  if (env.mode === "test") return "mock";
  if (env.dev && new URLSearchParams(env.search).has("mock")) return "mock";
  return "aevatar";
}

export class DelegatingAssistantTransport implements AssistantTransport {
  private transport: AssistantTransport;
  private interceptorInstalled = false;

  constructor(delegate: AssistantTransport) {
    this.transport = delegate;
  }

  current(): AssistantTransport {
    return this.transport;
  }

  install(transport: AssistantTransport): boolean {
    if (this.interceptorInstalled) return false;
    this.transport = transport;
    this.interceptorInstalled = true;
    return true;
  }

  listConversations(): Promise<Conversation[]> {
    return this.transport.listConversations();
  }

  createConversation(): Promise<Conversation> {
    return this.transport.createConversation();
  }

  getHistory(conversationId: string): Promise<ConversationHistory> {
    return this.transport.getHistory(conversationId);
  }

  reconcileProjection(
    conversationId: string,
  ): Promise<ProjectionReconcileOutcome> {
    return this.transport.reconcileProjection(conversationId);
  }

  releaseProjectionWaiter(conversationId: string): void {
    this.transport.releaseProjectionWaiter(conversationId);
  }

  deleteConversation(conversationId: string): Promise<void> {
    return this.transport.deleteConversation(conversationId);
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    return this.transport.sendMessage(conversationId, content, onEvent);
  }

  cancelActiveTurn(conversationId: string): void {
    this.transport.cancelActiveTurn(conversationId);
  }

  decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
    onEvent?: (event: TurnEvent) => void,
  ): Promise<TurnHandle | null> {
    return this.transport.decideApproval(
      conversationId,
      blockId,
      approved,
      onEvent,
    );
  }

  setActionCardInProgress(
    conversationId: string,
    blockId: string,
    inProgress: boolean,
    onEvent?: (event: TurnEvent) => void,
  ): void {
    this.transport.setActionCardInProgress(
      conversationId,
      blockId,
      inProgress,
      onEvent,
    );
  }

  blockActionCard(
    conversationId: string,
    blockId: string,
    note: string,
    onEvent?: (event: TurnEvent) => void,
  ): void {
    this.transport.blockActionCard(conversationId, blockId, note, onEvent);
  }

  continueActions(
    conversationId: string,
    originTurnId: string,
    reports: readonly ActionReport[],
    onEvent?: (event: TurnEvent) => void,
  ): TurnHandle | null {
    return this.transport.continueActions(
      conversationId,
      originTurnId,
      reports,
      onEvent,
    );
  }

  wakeActions(
    conversationId: string,
    originTurnId: string,
    onEvent?: (event: TurnEvent) => void,
  ): TurnHandle {
    return this.transport.wakeActions(conversationId, originTurnId, onEvent);
  }
}

export interface AssistantTransportFactories {
  readonly createMock: () => AssistantTransport;
  readonly createAevatar: () => AssistantTransport;
}

export interface AssistantInterceptorModule {
  readonly installScenarioInterceptor: (
    shell: DelegatingAssistantTransport,
  ) => void;
}

export type AssistantInterceptorLoader =
  () => Promise<AssistantInterceptorModule>;

export type AssistantMockScenariosStoreModule =
  typeof import("@/stores/assistant-mock-scenarios-store");

export type AssistantMockScenariosStoreLoader =
  () => Promise<AssistantMockScenariosStoreModule>;

const interceptorInstallations = new WeakMap<
  DelegatingAssistantTransport,
  Promise<void>
>();

export function installAssistantTransportInterceptor(
  shell: DelegatingAssistantTransport,
  loader: AssistantInterceptorLoader,
): Promise<void> {
  const existing = interceptorInstallations.get(shell);
  if (existing) return existing;
  const installation = loader().then((module) => {
    module.installScenarioInterceptor(shell);
  });
  // A failed chunk load must not poison the shell for the rest of the tab:
  // drop the rejected entry so a later flag resolution can retry. The returned
  // promise still rejects for this caller.
  installation.catch(() => {
    if (interceptorInstallations.get(shell) === installation) {
      interceptorInstallations.delete(shell);
    }
  });
  interceptorInstallations.set(shell, installation);
  return installation;
}

export function createAssistantTransportForEnvironment(
  env: {
    readonly mode: string;
    readonly dev: boolean;
    readonly search: string;
  },
  factories: AssistantTransportFactories,
): AssistantTransport {
  const kind = selectAssistantTransportKind(env);
  if (kind === "mock") return factories.createMock();
  return new DelegatingAssistantTransport(factories.createAevatar());
}

export interface AssistantScenarioFeatureLoaders {
  readonly loadInterceptor: AssistantInterceptorLoader;
  readonly loadStore: AssistantMockScenariosStoreLoader;
}

const defaultScenarioFeatureLoaders: AssistantScenarioFeatureLoaders = {
  loadInterceptor: () => import("@/lib/assistant/scenario-intercept-transport"),
  loadStore: () => import("@/stores/assistant-mock-scenarios-store"),
};

/**
 * Apply the resolved `experimental:assistant-mock-scenarios` value to the live
 * transport. Called by the React gate on the assistant page; this is the only
 * path that installs the scenario interceptor.
 *
 * Off (the default for every user) is a hard no-op the first time through: the
 * interceptor, engine, scenario config, and persisted store all stay in unloaded
 * async chunks, so a flagless session never fetches them and the live transport
 * is untouched. Once a tab has armed the interceptor, flipping the flag off
 * still has to reach it, so from then on the off path loads the store to clear
 * `featureEnabled` — the second gate `ScenarioInterceptTransport.sendMessage`
 * checks alongside the user's own toggle.
 *
 * Never rejects: a chunk that fails to load reports through `engineState` (the
 * popover surfaces it) rather than throwing into a `useEffect`.
 */
export async function applyAssistantScenarioFeature(
  featureEnabled: boolean,
  transport: AssistantTransport = assistantTransport,
  loaders: AssistantScenarioFeatureLoaders = defaultScenarioFeatureLoaders,
): Promise<void> {
  // Full-mock selection (`?mock=1` / test mode) has no shell to wrap.
  if (!(transport instanceof DelegatingAssistantTransport)) return;
  const armed = interceptorInstallations.has(transport);
  if (!featureEnabled && !armed) return;

  let store: AssistantMockScenariosStoreModule;
  try {
    store = await loaders.loadStore();
  } catch {
    return;
  }
  const state = () => store.useAssistantMockScenariosStore.getState();
  state().setFeatureEnabled(featureEnabled);
  if (!featureEnabled) return;
  if (!armed) state().setEngineState("loading");
  try {
    await installAssistantTransportInterceptor(
      transport,
      loaders.loadInterceptor,
    );
  } catch {
    state().setEngineState("error");
  }
}

function createAssistantTransport(): AssistantTransport {
  return createAssistantTransportForEnvironment(
    {
      mode: import.meta.env.MODE,
      dev: import.meta.env.DEV,
      search: typeof window === "undefined" ? "" : window.location.search,
    },
    {
      createMock: () => new MockAssistantTransport(),
      createAevatar: () => new AevatarAssistantTransport(),
    },
  );
}

export const assistantTransport: AssistantTransport =
  createAssistantTransport();

export function resetAssistantTransport(now: () => number = Date.now): void {
  if (assistantTransport instanceof MockAssistantTransport) {
    assistantTransport.reset(now);
  }
}
