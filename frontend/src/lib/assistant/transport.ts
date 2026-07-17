import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import { AevatarCompletionsTransport } from "@/lib/assistant/completions-transport";
import { AevatarWorkflowChatTransport } from "@/lib/assistant/workflow-transport";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import {
  assistantMockStore,
  createScriptedTurn,
  resetAssistantMockStore,
} from "@/lib/assistant/mock-data";
import { toTerminalBlock } from "@/lib/assistant/stream";
import { useAssistantTransportStore } from "@/stores/assistant-transport-store";
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

class MockAssistantTransport implements AssistantTransport {
  private readonly running = new Map<string, RunningScript>();

  async listConversations(): Promise<Conversation[]> {
    return assistantMockStore.listConversations();
  }

  async createConversation(): Promise<Conversation> {
    return assistantMockStore.createConversation();
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    return assistantMockStore.getHistory(conversationId);
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

    const emit = (event: TurnEvent) => {
      if (script.cancelled || script.finished) return;
      assistantMockStore.applyEvent(conversationId, event);
      this.trackLifecycle(script, event);
      onEvent(event);
      if (event.event === "turn.completed") {
        script.finished = true;
        this.running.delete(conversationId);
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

  async decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
  ): Promise<void> {
    assistantMockStore.decideApproval(conversationId, blockId, approved);
  }

  reset(now: () => number = Date.now): void {
    for (const script of this.running.values()) {
      script.cancelled = true;
      for (const timer of script.timers) clearTimeout(timer);
    }
    this.running.clear();
    resetAssistantMockStore(now);
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
  }
}

/**
 * Routes every call to whichever aevatar API the sidebar's "Chat API"
 * toggle currently selects. The target is resolved per call rather than
 * captured: the store can flip between any of them.
 */
export class ModeSwitchingTransport implements AssistantTransport {
  private readonly chat: AssistantTransport;
  private readonly completions: AssistantTransport;
  private readonly workflow: AssistantTransport;

  constructor(
    chat: AssistantTransport = new AevatarAssistantTransport(),
    completions: AssistantTransport = new AevatarCompletionsTransport(),
    workflow: AssistantTransport = new AevatarWorkflowChatTransport(),
  ) {
    this.chat = chat;
    this.completions = completions;
    this.workflow = workflow;
  }

  private target(): AssistantTransport {
    switch (useAssistantTransportStore.getState().mode) {
      case "completions":
        return this.completions;
      case "workflow":
        return this.workflow;
      default:
        return this.chat;
    }
  }

  listConversations(): Promise<Conversation[]> {
    return this.target().listConversations();
  }

  createConversation(): Promise<Conversation> {
    return this.target().createConversation();
  }

  getHistory(conversationId: string): Promise<ConversationHistory> {
    return this.target().getHistory(conversationId);
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    // The handle closes over the transport that started the turn, so a
    // mid-stream toggle cannot misroute the cancel.
    return this.target().sendMessage(conversationId, content, onEvent);
  }

  decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
  ): Promise<void> {
    return this.target().decideApproval(conversationId, blockId, approved);
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
    : new ModeSwitchingTransport();
}

export const assistantTransport: AssistantTransport =
  createAssistantTransport();

export function resetAssistantTransport(now: () => number = Date.now): void {
  if (assistantTransport instanceof MockAssistantTransport) {
    assistantTransport.reset(now);
  }
}
