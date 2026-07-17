import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import { drainSseBuffer } from "@/lib/assistant/sse";
import {
  applyTurnEvent,
  EMPTY_TURN_STATE,
  toTerminalBlock,
} from "@/lib/assistant/stream";
import type {
  AssistantMessage,
  AssistantTransport,
  Conversation,
  ConversationHistory,
  TurnEvent,
  TurnHandle,
  TurnReducerState,
} from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";

// NyxID's own assistant pass-through, which forwards to aevatar's
// OpenAI-compatible surface through the admin-managed service (PRD decision
// 4). The browser never addresses aevatar directly. Unlike nyxid-chat this
// surface is stateless: no server-side conversations, so every turn carries
// the full message history.
const COMPLETIONS_URL = "/api/v1/assistant/completions";

const MAX_MESSAGE_CHARS = 32_768;
const TITLE_CHARS = 40;

/** OpenAI `chat.completion.chunk` frames observed on the live stream. */
interface CompletionChunk {
  readonly id?: string;
  readonly choices?: ReadonlyArray<{
    readonly delta?: { readonly content?: string | null };
    readonly finish_reason?: string | null;
  }>;
  readonly error?: { readonly code?: string; readonly message?: string };
}

interface OpenAiMessage {
  readonly role: "user" | "assistant";
  readonly content: string;
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

/** Flattens the local transcript into OpenAI `{role, content}` pairs. */
function toOpenAiMessages(messages: readonly AssistantMessage[]): OpenAiMessage[] {
  const payload: OpenAiMessage[] = [];
  for (const message of messages) {
    if (message.role !== "user" && message.role !== "assistant") continue;
    const content = message.blocks
      .filter((block) => block.type === "text")
      .map((block) => (block.type === "text" ? block.text : ""))
      .filter((text) => text.length > 0)
      .join("\n\n");
    if (!content) continue;
    payload.push({ role: message.role, content });
  }
  return payload;
}

/**
 * `AssistantTransport` backed by aevatar's OpenAI-compatible
 * `/v1/chat/completions`. The endpoint has no conversation concept, so the
 * conversation list and every transcript live in this in-memory store and
 * reset on reload — accepted for this experimental surface. Conversation ids
 * are prefixed `completions-` so they never collide with the nyxid-chat
 * transport's server-issued ids; a cross-mode lookup fails closed.
 */
export class AevatarCompletionsTransport implements AssistantTransport {
  private readonly conversations = new Map<string, StoredConversation>();
  private readonly running = new Map<string, RunningTurn>();

  async listConversations(): Promise<Conversation[]> {
    return [...this.conversations.values()]
      .map((stored) => stored.conversation)
      .sort((a, b) => b.last_message_at.localeCompare(a.last_message_at));
  }

  async createConversation(): Promise<Conversation> {
    const createdAt = new Date().toISOString();
    const conversation: Conversation = {
      id: newId("completions"),
      title: "New chat",
      created_at: createdAt,
      last_message_at: createdAt,
    };
    this.conversations.set(conversation.id, {
      conversation,
      turnState: EMPTY_TURN_STATE,
    });
    return conversation;
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    const stored = this.conversations.get(conversationId);
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
        ? normalized.slice(0, TITLE_CHARS)
        : stored.conversation.title,
      last_message_at: createdAt,
    };
    // Snapshot before streaming: the request body is the transcript as of
    // this send, new user message last.
    const requestMessages = toOpenAiMessages(stored.turnState.messages);

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
    void this.streamTurn(conversationId, run, requestMessages);
    return {
      turnId: run.turnId,
      cancel: () => {
        this.cancelTurn(conversationId, run);
      },
    };
  }

  // Parameterless (still structurally an `AssistantTransport`): the
  // completions API surfaces no tool calls, so an approval card can never
  // reach this transport's transcript.
  async decideApproval(): Promise<void> {
    throw new Error("Approvals are not available on the completions API.");
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
    messages: readonly OpenAiMessage[],
  ): Promise<void> {
    try {
      // Hand-rolled fetch (not `apiClient`): the response is an SSE stream,
      // and the endpoint 415s without an explicit JSON content type. `model`
      // is omitted so the server picks its default.
      const response = await fetch(COMPLETIONS_URL, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "text/event-stream",
        },
        credentials: "include",
        body: JSON.stringify({ messages, stream: true }),
        signal: run.controller.signal,
      });
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
          this.handleChunk(conversationId, run, payload);
          if (run.finished) return;
        }
      }
      // Stream closed without a terminal chunk or `[DONE]`: settle rather
      // than leaving the turn spinning forever. An empty completion lands
      // here with nothing open, and still settles `completed`.
      this.closeOpenMessage(conversationId, run);
      this.finishTurn(conversationId, run, "completed", null);
    } catch (error) {
      // The cancel path aborts the fetch after emitting its own terminal
      // events; an abort here is not a failure.
      if (run.finished || run.controller.signal.aborted) return;
      this.closeOpenMessage(conversationId, run);
      this.finishTurn(conversationId, run, "failed", {
        code: "network_error",
        message: error instanceof Error ? error.message : "The stream failed.",
      });
    }
  }

  private handleChunk(
    conversationId: string,
    run: RunningTurn,
    payload: string,
  ): void {
    // The terminal sentinel is not JSON, so it must be matched before parse.
    if (payload === "[DONE]") {
      this.closeOpenMessage(conversationId, run);
      this.finishTurn(conversationId, run, "completed", null);
      return;
    }
    let chunk: CompletionChunk;
    try {
      chunk = JSON.parse(payload) as CompletionChunk;
    } catch {
      // Unparseable frames are skipped: never drop the turn over one.
      return;
    }
    if (chunk.error) {
      this.closeOpenMessage(conversationId, run);
      this.finishTurn(conversationId, run, "failed", {
        code: chunk.error.code ?? "completions_error",
        message: chunk.error.message ?? "The assistant run failed.",
      });
      return;
    }
    const choice = chunk.choices?.[0];
    const delta = choice?.delta?.content;
    if (delta) {
      if (!run.currentBlockId) {
        const messageId = chunk.id ?? newId("assistant-message");
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
      }
      run.accumulatedText += delta;
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.delta",
        block_id: run.currentBlockId,
        text: delta,
      });
    }
    if (choice?.finish_reason) {
      this.closeOpenMessage(conversationId, run);
      this.finishTurn(conversationId, run, "completed", null);
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
   * Client-side stop: the completions endpoint has no cancel channel, so
   * cancelling aborts the SSE fetch and settles the local turn with whatever
   * text arrived. Nothing survives server-side to reconcile against.
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
