import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import { drainSseBuffer } from "@/lib/assistant/sse";
import { applyTurnEvent, EMPTY_TURN_STATE } from "@/lib/assistant/stream";
import type {
  AssistantTransport,
  Conversation,
  ConversationHistory,
  TurnEvent,
  TurnHandle,
  TurnReducerState,
} from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";

// NyxID's assistant pass-through onto aevatar's ad-hoc workflow chat
// (`StartWorkflowChat`). Each send starts a fresh workflow run carrying only
// that prompt — the surface has no history concept, and the stream is the
// raw workflow engine event feed, from which this transport renders only the
// final run output. The telemetry envelopes (workflow YAML, system prompts,
// kernel state) still reach the browser: PRD §5.8's exclusion was expressly
// waived for this surface (2026-07-17).
const WORKFLOW_CHAT_URL = "/api/v1/assistant/workflow-chat";

const MAX_MESSAGE_CHARS = 32_768;
const TITLE_CHARS = 40;

/** Frames observed on the live `/api/chat` stream (2026-07-17 capture). */
interface WorkflowEnvelope {
  readonly custom?: {
    readonly name?: string;
    readonly payload?: unknown;
  };
  readonly runFinished?: {
    readonly result?: { readonly output?: string };
  };
  readonly runError?: {
    readonly code?: string;
    readonly message?: string;
  };
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
  finished: boolean;
}

function newId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

/**
 * `AssistantTransport` backed by aevatar's workflow chat. Like the
 * completions transport, conversations live only in this in-memory store and
 * reset on reload; ids are prefixed `workflow-` so a cross-mode lookup fails
 * closed. Unlike completions, the upstream takes a single `prompt` per run —
 * previous turns are shown locally but not resent.
 */
export class AevatarWorkflowChatTransport implements AssistantTransport {
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
      id: newId("workflow"),
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

    const run: RunningTurn = {
      turnId: newId("turn"),
      controller: new AbortController(),
      onEvent,
      cursor: 0,
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

  // Parameterless (still structurally an `AssistantTransport`): the workflow
  // chat run is fire-and-forget, so an approval card can never reach this
  // transport's transcript.
  async decideApproval(): Promise<void> {
    throw new Error("Approvals are not available on the workflow chat API.");
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
      // Hand-rolled fetch (not `apiClient`): the response is an SSE stream.
      const response = await fetch(WORKFLOW_CHAT_URL, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "text/event-stream",
        },
        credentials: "include",
        body: JSON.stringify({ prompt }),
        signal: run.controller.signal,
      });
      if (!response.ok || !response.body) {
        this.finishTurn(conversationId, run, "failed", {
          code: `http_${String(response.status)}`,
          message: "The workflow chat stream could not be started.",
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
          this.handleEnvelope(conversationId, run, payload);
          if (run.finished) return;
        }
      }
      // The engine feed closed without a `runFinished`: there is no output
      // to render, so the turn failed rather than quietly completing empty.
      this.finishTurn(conversationId, run, "failed", {
        code: "stream_ended",
        message: "The workflow run ended without a result.",
      });
    } catch (error) {
      // The cancel path aborts the fetch after emitting its own terminal
      // events; an abort here is not a failure.
      if (run.finished || run.controller.signal.aborted) return;
      this.finishTurn(conversationId, run, "failed", {
        code: "network_error",
        message: error instanceof Error ? error.message : "The stream failed.",
      });
    }
  }

  private handleEnvelope(
    conversationId: string,
    run: RunningTurn,
    payload: string,
  ): void {
    let envelope: WorkflowEnvelope;
    try {
      envelope = JSON.parse(payload) as WorkflowEnvelope;
    } catch {
      // Unparseable frames are skipped: never drop the turn over one.
      return;
    }
    if (envelope.runError) {
      this.finishTurn(conversationId, run, "failed", {
        code: envelope.runError.code ?? "workflow_error",
        message: envelope.runError.message ?? "The workflow run failed.",
      });
      return;
    }
    // Everything before `runFinished` — `aevatar.run.context`, the
    // `aevatar.raw.observed` telemetry, step events — carries engine state,
    // not chat content. Only the final run output is rendered.
    if (!envelope.runFinished) return;
    const output = envelope.runFinished.result?.output ?? "";
    if (output) {
      const messageId = newId("assistant-message");
      const blockId = `${messageId}-text`;
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
        block_id: blockId,
        index: 0,
        block: { type: "text", block_id: blockId, text: "" },
      });
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.delta",
        block_id: blockId,
        text: output,
      });
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.completed",
        block_id: blockId,
        block: { type: "text", block_id: blockId, text: output },
      });
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "message.completed",
        message_id: messageId,
      });
    }
    this.finishTurn(conversationId, run, "completed", null);
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
   * Client-side stop: the workflow chat endpoint has no cancel channel, so
   * cancelling aborts the SSE fetch and settles the local turn. The workflow
   * run itself keeps executing server-side; its output is simply dropped.
   */
  private cancelTurn(conversationId: string, run: RunningTurn): void {
    if (run.finished) return;
    run.controller.abort();
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.status",
      turn_id: run.turnId,
      status: "cancelled",
    });
    this.finishTurn(conversationId, run, "cancelled", null);
  }
}
