import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import {
  composeUnreportedCompletedNote,
} from "@/lib/assistant/action-notes";
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
  type ActionReport,
} from "@/schemas/assistant-actions";
import type {
  AssistantTransport,
  Conversation,
  ConversationHistory,
  TurnEvent,
  TurnHandle,
} from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";

export { AssistantTurnActiveError };

const EVENT_CADENCE_MS = 100;

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
    return assistantMockStore.createConversation();
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    return assistantMockStore.getHistory(conversationId);
  }

  async deleteConversation(conversationId: string): Promise<void> {
    const script = this.running.get(conversationId);
    if (script) this.cancelScript(conversationId, script);
    this.pendingActions.delete(conversationId);
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
    const turnId = assistantMockStore.nextId("turn");
    const messageId = assistantMockStore.nextId("assistant-message");
    const blockId = assistantMockStore.nextId("assistant-block");
    const events = createScriptedTurn(turnId, messageId, blockId);
    return this.startScript(conversationId, turnId, messageId, events, onEvent);
  }

  cancelActiveTurn(conversationId: string): void {
    const script = this.running.get(conversationId);
    if (script) this.cancelScript(conversationId, script);
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
    const declined = batch.reports.every(
      (report) => report.disposition === "declined",
    );
    const text = declined
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

function createAssistantTransport(): AssistantTransport {
  const kind = selectAssistantTransportKind({
    mode: import.meta.env.MODE,
    dev: import.meta.env.DEV,
    search: typeof window === "undefined" ? "" : window.location.search,
  });
  return kind === "mock"
    ? new MockAssistantTransport()
    : new AevatarAssistantTransport();
}

export const assistantTransport: AssistantTransport =
  createAssistantTransport();

export function resetAssistantTransport(now: () => number = Date.now): void {
  if (assistantTransport instanceof MockAssistantTransport) {
    assistantTransport.reset(now);
  }
}
