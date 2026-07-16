import { api } from "@/lib/api-client";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import {
  applyTurnEvent,
  EMPTY_TURN_STATE,
  toTerminalBlock,
} from "@/lib/assistant/stream";
import { useAuthStore } from "@/stores/auth-store";
import type {
  ApprovalCardContentBlock,
  AssistantMessage,
  AssistantTransport,
  Conversation,
  ConversationHistory,
  TurnEvent,
  TurnHandle,
  TurnReducerState,
} from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";

// Aevatar's nyxid-chat surface, reached through the NyxID proxy so the
// browser session authenticates and the broker injects the `scope_id`
// claim Aevatar resolves the scope from. The scope id is the caller's
// NyxID subject id, so the path segment must match the signed-in user.
const AEVATAR_PROXY_PREFIX = "/proxy/s/aevatar";

// The conversation list endpoint returns bare actor ids, so titles and
// timestamps come from per-conversation history hydration. Cap the
// first-load fan-out; conversations beyond the cap render with a
// placeholder title until opened.
const MAX_TITLE_HYDRATIONS = 20;
// Sidebar list re-fetch throttle: `projectTransportState` re-projects the
// conversation list after every turn event, which must not become one
// network round-trip per streamed token.
const CONVERSATION_LIST_TTL_MS = 5_000;

const MAX_MESSAGE_CHARS = 32_768;

/** AG-UI protocol frames observed on `nyxid-chat/conversations/{id}:stream`. */
interface AgUiFrame {
  readonly type?: string;
  readonly textMessageStart?: {
    readonly messageId?: string;
    readonly role?: string;
  };
  readonly textMessageContent?: { readonly delta?: string };
  readonly textMessageEnd?: { readonly messageId?: string };
  readonly error?: { readonly code?: string; readonly message?: string };
  readonly message?: string;
}

interface AevatarConversationListResponse {
  readonly conversations?: Array<{ readonly actorId?: string }>;
}

interface AevatarCreateConversationResponse {
  readonly actorId?: string;
}

interface AevatarHistoryEntry {
  readonly id?: string;
  readonly role?: string;
  readonly content?: string | null;
  readonly timestamp?: number;
}

interface StoredConversation {
  conversation: Conversation;
  turnState: TurnReducerState;
}

interface RunningTurn {
  readonly turnId: string;
  readonly controller: AbortController;
  readonly onEvent: (event: TurnEvent) => void;
  cursor: number;
  currentMessageId: string | null;
  currentBlockId: string | null;
  accumulatedText: string;
  finished: boolean;
}

function newId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

function isoFromEpochMs(epochMs: number | undefined, fallback: string): string {
  if (typeof epochMs !== "number" || !Number.isFinite(epochMs)) {
    return fallback;
  }
  return new Date(epochMs).toISOString();
}

/**
 * Incremental SSE framing: consumes complete `data:` payloads from the
 * buffer and returns the unterminated remainder for the next read.
 */
export function drainSseBuffer(buffer: string): {
  readonly payloads: string[];
  readonly rest: string;
} {
  const normalized = buffer.replace(/\r\n/g, "\n");
  const segments = normalized.split("\n\n");
  const rest = segments.pop() ?? "";
  const payloads: string[] = [];
  for (const segment of segments) {
    const data = segment
      .split("\n")
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice("data:".length).trimStart())
      .join("\n");
    if (data) payloads.push(data);
  }
  return { payloads, rest };
}

function historyEntryToMessage(
  entry: AevatarHistoryEntry,
  index: number,
): AssistantMessage | null {
  if (entry.role !== "user" && entry.role !== "assistant") return null;
  const id = entry.id ?? `history-${String(index)}`;
  const text = entry.content ?? "";
  return {
    id,
    role: entry.role,
    schema_version: 1,
    blocks: text
      ? [{ type: "text", block_id: `${id}-text`, text }]
      : [],
    created_at: isoFromEpochMs(entry.timestamp, new Date(0).toISOString()),
  };
}

function deriveTitle(messages: AssistantMessage[]): string | null {
  const firstUser = messages.find((message) => message.role === "user");
  const firstText = firstUser?.blocks.find((block) => block.type === "text");
  if (!firstText || firstText.type !== "text" || !firstText.text.trim()) {
    return null;
  }
  return firstText.text.trim().slice(0, 40);
}

/**
 * Real `AssistantTransport` backed by Aevatar's nyxid-chat API (PRD C1
 * provider). Conversation state is mirrored in a client-side store so
 * streaming turns render live from turn events while the server history
 * (`chat-history/conversations/{id}`) stays authoritative between turns —
 * the same authority split the mock transport established.
 */
export class AevatarAssistantTransport implements AssistantTransport {
  private readonly conversations = new Map<string, StoredConversation>();
  private readonly running = new Map<string, RunningTurn>();
  private listFetchedAt = 0;

  private scopePath(suffix: string): string {
    const userId = useAuthStore.getState().user?.id;
    if (!userId) {
      throw new Error("You must be signed in to use the assistant.");
    }
    return `${AEVATAR_PROXY_PREFIX}/api/scopes/${userId}/${suffix}`;
  }

  async listConversations(): Promise<Conversation[]> {
    const now = Date.now();
    if (this.running.size === 0 && now - this.listFetchedAt > CONVERSATION_LIST_TTL_MS) {
      this.listFetchedAt = now;
      const response = await api.get<AevatarConversationListResponse>(
        this.scopePath("nyxid-chat/conversations"),
      );
      const serverIds = (response.conversations ?? [])
        .map((item) => item.actorId)
        .filter((id): id is string => Boolean(id));
      const unknown = serverIds.filter((id) => !this.conversations.has(id));
      await Promise.all(
        unknown.slice(0, MAX_TITLE_HYDRATIONS).map(async (id) => {
          try {
            await this.loadHistory(id);
          } catch {
            this.seedPlaceholder(id);
          }
        }),
      );
      for (const id of unknown.slice(MAX_TITLE_HYDRATIONS)) {
        this.seedPlaceholder(id);
      }
    }
    return [...this.conversations.values()]
      .map((stored) => stored.conversation)
      .sort((a, b) => b.last_message_at.localeCompare(a.last_message_at));
  }

  async createConversation(): Promise<Conversation> {
    const response = await api.post<AevatarCreateConversationResponse>(
      this.scopePath("nyxid-chat/conversations"),
      {},
    );
    const id = response.actorId;
    if (!id) {
      throw new Error("Aevatar did not return a conversation id.");
    }
    const createdAt = new Date().toISOString();
    const conversation: Conversation = {
      id,
      title: "New chat",
      created_at: createdAt,
      last_message_at: createdAt,
    };
    this.conversations.set(id, {
      conversation,
      turnState: EMPTY_TURN_STATE,
    });
    return conversation;
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    const existing = this.conversations.get(conversationId);
    // During a streaming turn the local mirror is ahead of the server;
    // serving it keeps per-event re-projection off the network entirely.
    if (existing && isTurnActive(existing.turnState.activeTurn?.status)) {
      return {
        conversation: existing.conversation,
        messages: existing.turnState.messages,
        has_more: false,
      };
    }
    let stored: StoredConversation | undefined;
    try {
      stored = await this.loadHistory(conversationId);
    } catch {
      stored = existing;
    }
    if (!stored) {
      throw new Error("Conversation was not found.");
    }
    return {
      conversation: stored.conversation,
      messages: stored.turnState.messages,
      has_more: false,
    };
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    const stored = this.conversations.get(conversationId);
    if (!stored) {
      throw new Error("Conversation was not found.");
    }
    if (
      this.running.has(conversationId) ||
      isTurnActive(stored.turnState.activeTurn?.status)
    ) {
      throw new AssistantTurnActiveError();
    }
    const normalized = content.trim();
    if (!normalized || normalized.length > MAX_MESSAGE_CHARS) {
      throw new Error("Message must contain between 1 and 32768 characters.");
    }

    // Optimistic user message; the server materializes its own copy, which
    // replaces this one on the post-turn history reload.
    const createdAt = new Date().toISOString();
    const firstMessage = stored.turnState.messages.length === 0;
    stored.turnState = {
      ...stored.turnState,
      messages: [
        ...stored.turnState.messages,
        {
          id: newId("user-message"),
          role: "user",
          schema_version: 1,
          blocks: [
            {
              type: "text",
              block_id: newId("user-block"),
              text: normalized,
            },
          ],
          created_at: createdAt,
        },
      ],
      // Cursors restart at 1 for each turn's event stream.
      lastCursor: 0,
    };
    stored.conversation = {
      ...stored.conversation,
      title: firstMessage
        ? normalized.slice(0, 40)
        : stored.conversation.title,
      last_message_at: createdAt,
    };

    const run: RunningTurn = {
      turnId: newId("turn"),
      controller: new AbortController(),
      onEvent,
      cursor: 0,
      currentMessageId: null,
      currentBlockId: null,
      accumulatedText: "",
      finished: false,
    };
    this.running.set(conversationId, run);
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.status",
      turn_id: run.turnId,
      status: "running",
    });
    void this.streamTurn(conversationId, run, normalized);
    return {
      turnId: run.turnId,
      cancel: () => {
        this.cancelTurn(conversationId, run);
      },
    };
  }

  async decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
  ): Promise<void> {
    const stored = this.conversations.get(conversationId);
    const card = stored?.turnState.messages
      .flatMap((message) => message.blocks)
      .find(
        (block): block is ApprovalCardContentBlock =>
          block.type === "approval_card" && block.block_id === blockId,
      );
    if (!card) {
      throw new Error("Approval request was not found.");
    }
    await api.post(
      this.scopePath(`nyxid-chat/conversations/${conversationId}:approve`),
      { requestId: card.approval_request_id, approved },
    );
  }

  private seedPlaceholder(id: string): void {
    if (this.conversations.has(id)) return;
    const seededAt = new Date(0).toISOString();
    this.conversations.set(id, {
      conversation: {
        id,
        title: "Conversation",
        created_at: seededAt,
        last_message_at: seededAt,
      },
      turnState: EMPTY_TURN_STATE,
    });
  }

  private async loadHistory(
    conversationId: string,
  ): Promise<StoredConversation> {
    const entries = await api.get<AevatarHistoryEntry[]>(
      this.scopePath(`chat-history/conversations/${conversationId}`),
    );
    const existing = this.conversations.get(conversationId);
    const messages = entries
      .map((entry, index) => historyEntryToMessage(entry, index))
      .filter((message): message is AssistantMessage => message !== null);
    // Keep-max guard: immediately after a turn completes, the server-side
    // materialization can briefly lag the local mirror. Never replace a
    // richer local transcript with a shorter server one.
    if (existing && existing.turnState.messages.length > messages.length) {
      return existing;
    }
    const first = messages[0];
    const last = messages[messages.length - 1];
    const nowIso = new Date().toISOString();
    const conversation: Conversation = {
      id: conversationId,
      title:
        deriveTitle(messages) ?? existing?.conversation.title ?? "Conversation",
      created_at:
        first?.created_at ?? existing?.conversation.created_at ?? nowIso,
      last_message_at:
        last?.created_at ?? existing?.conversation.last_message_at ?? nowIso,
    };
    const stored: StoredConversation = {
      conversation,
      turnState: {
        messages,
        activeTurn: existing?.turnState.activeTurn ?? null,
        lastCursor: existing?.turnState.lastCursor ?? 0,
      },
    };
    this.conversations.set(conversationId, stored);
    return stored;
  }

  private nextCursor(run: RunningTurn): number {
    run.cursor += 1;
    return run.cursor;
  }

  private emit(
    conversationId: string,
    run: RunningTurn,
    event: TurnEvent,
  ): void {
    if (run.finished) return;
    const stored = this.conversations.get(conversationId);
    if (stored) {
      stored.turnState = applyTurnEvent(stored.turnState, event);
      stored.conversation = {
        ...stored.conversation,
        last_message_at: new Date().toISOString(),
      };
    }
    if (event.event === "turn.completed") {
      run.finished = true;
      this.running.delete(conversationId);
    }
    run.onEvent(event);
  }

  private async streamTurn(
    conversationId: string,
    run: RunningTurn,
    prompt: string,
  ): Promise<void> {
    try {
      // Hand-rolled fetch (not `apiClient`): the response is an SSE stream,
      // and the endpoint 415s without an explicit JSON content type.
      const response = await fetch(
        `/api/v1${this.scopePath(
          `nyxid-chat/conversations/${conversationId}:stream`,
        )}`,
        {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Accept: "text/event-stream",
          },
          credentials: "include",
          body: JSON.stringify({ prompt }),
          signal: run.controller.signal,
        },
      );
      if (!response.ok || !response.body) {
        this.finishTurn(conversationId, run, "failed", {
          code: `http_${String(response.status)}`,
          message: "The assistant stream could not be started.",
        });
        return;
      }
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const { payloads, rest } = drainSseBuffer(buffer);
        buffer = rest;
        for (const payload of payloads) {
          this.handleAgUiFrame(conversationId, run, payload);
          if (run.finished) return;
        }
      }
      // Stream closed without a RUN_FINISHED frame: settle rather than
      // leaving the turn spinning forever.
      this.closeOpenMessage(conversationId, run);
      this.finishTurn(conversationId, run, "completed", null);
    } catch (error) {
      // The cancel path aborts the fetch after emitting its own terminal
      // events; an abort here is not a failure.
      if (run.finished || run.controller.signal.aborted) return;
      this.closeOpenMessage(conversationId, run);
      this.finishTurn(conversationId, run, "failed", {
        code: "network_error",
        message:
          error instanceof Error ? error.message : "The stream failed.",
      });
    }
  }

  private handleAgUiFrame(
    conversationId: string,
    run: RunningTurn,
    payload: string,
  ): void {
    let frame: AgUiFrame;
    try {
      frame = JSON.parse(payload) as AgUiFrame;
    } catch {
      return;
    }
    switch (frame.type) {
      case "TEXT_MESSAGE_START": {
        const messageId =
          frame.textMessageStart?.messageId ?? newId("assistant-message");
        run.currentMessageId = messageId;
        run.currentBlockId = `${messageId}-text`;
        run.accumulatedText = "";
        this.emit(conversationId, run, {
          cursor: this.nextCursor(run),
          event: "message.started",
          message_id: messageId,
          role: "assistant",
        });
        this.emit(conversationId, run, {
          cursor: this.nextCursor(run),
          event: "block.started",
          message_id: messageId,
          block_id: run.currentBlockId,
          index: 0,
          block: { type: "text", block_id: run.currentBlockId, text: "" },
        });
        return;
      }
      case "TEXT_MESSAGE_CONTENT": {
        const delta = frame.textMessageContent?.delta ?? "";
        if (!delta || !run.currentBlockId) return;
        run.accumulatedText += delta;
        this.emit(conversationId, run, {
          cursor: this.nextCursor(run),
          event: "block.delta",
          block_id: run.currentBlockId,
          text: delta,
        });
        return;
      }
      case "TEXT_MESSAGE_END": {
        this.closeOpenMessage(conversationId, run);
        return;
      }
      case "RUN_ERROR": {
        this.closeOpenMessage(conversationId, run);
        this.finishTurn(conversationId, run, "failed", {
          code: frame.error?.code ?? "run_error",
          message:
            frame.error?.message ??
            frame.message ??
            "The assistant run failed.",
        });
        return;
      }
      case "RUN_FINISHED": {
        this.closeOpenMessage(conversationId, run);
        this.finishTurn(conversationId, run, "completed", null);
        return;
      }
      default:
        // RUN_STARTED, USAGE, and any newer AG-UI frame types have no
        // presentation mapping; skipping them is the §5.9 forward-compat
        // posture (never drop the turn over an unknown frame).
        return;
    }
  }

  private closeOpenMessage(conversationId: string, run: RunningTurn): void {
    if (!run.currentMessageId || !run.currentBlockId) return;
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.completed",
      block_id: run.currentBlockId,
      block: {
        type: "text",
        block_id: run.currentBlockId,
        text: run.accumulatedText,
      },
    });
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "message.completed",
      message_id: run.currentMessageId,
    });
    run.currentMessageId = null;
    run.currentBlockId = null;
    run.accumulatedText = "";
  }

  private finishTurn(
    conversationId: string,
    run: RunningTurn,
    status: "completed" | "failed" | "cancelled",
    error: { code: string; message: string } | null,
  ): void {
    if (run.finished) return;
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.completed",
      turn_id: run.turnId,
      status,
      error,
    });
  }

  /**
   * Client-side stop: aevatar's nyxid-chat surface has no cancel endpoint,
   * so cancelling aborts the SSE fetch and settles the local turn per the
   * PRD stop-flow. The server-side run may still finish; its full reply
   * then surfaces on the next history reload.
   */
  private cancelTurn(conversationId: string, run: RunningTurn): void {
    if (run.finished) return;
    run.controller.abort();
    if (run.currentBlockId) {
      const stored = this.conversations.get(conversationId);
      const openBlock = stored?.turnState.messages
        .flatMap((message) => message.blocks)
        .find((block) => block.block_id === run.currentBlockId);
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.completed",
        block_id: run.currentBlockId,
        block: openBlock
          ? toTerminalBlock(openBlock)
          : {
              type: "text",
              block_id: run.currentBlockId,
              text: run.accumulatedText,
            },
      });
      if (run.currentMessageId) {
        this.emit(conversationId, run, {
          cursor: this.nextCursor(run),
          event: "message.completed",
          message_id: run.currentMessageId,
        });
      }
      run.currentMessageId = null;
      run.currentBlockId = null;
    }
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.status",
      turn_id: run.turnId,
      status: "cancelled",
    });
    this.finishTurn(conversationId, run, "cancelled", null);
  }
}
