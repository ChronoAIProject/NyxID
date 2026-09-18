import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { assistantHttp, assistantJson } from "@/lib/assistant/assistant-http";
import {
  applyDirectTurnEvent,
  drainDirectSseBuffer,
  type DirectTurnState,
} from "@/lib/assistant/direct-transport";
import { getAssistantIdentityUserId, subscribeAssistantIdentity } from "@/lib/assistant/identity";
import { isNyxAgentConversationId } from "@/lib/assistant/conversation-ids";
import type { ChatSessionState, ChatMessage } from "@/lib/assistant/chat-types";
import type { RuntimeToolCallInfo } from "@/lib/assistant/runtime-event-semantics";
import {
  nyxAgentAcknowledgementSchema,
  nyxAgentConversationSchema,
  nyxAgentHistorySchema,
  nyxAgentIndexSchema,
  nyxAgentModelsSchema,
  nyxAgentEventSchema,
  type NyxAgentAccessMode,
  type NyxAgentConversation,
  type NyxAgentHistory,
  type NyxAgentTurnActivity,
} from "@/schemas/assistant-nyxagent";

const ROOT = "/assistant/nyxagent";
export const CONTEXT_RESET_NOTICE =
  "Conversation context was reset; the assistant was given a recap of this chat.";

interface LiveTurn {
  state: DirectTurnState;
  conversation: NyxAgentConversation;
  controller: AbortController;
  resetBeforeMessageId?: string;
}

function toolCalls(
  activities: readonly NyxAgentTurnActivity[] | undefined,
): { toolCalls?: RuntimeToolCallInfo[] } {
  if (!activities?.length) return {};
  return {
    toolCalls: activities.map((activity) => ({
      id: activity.id,
      name: activity.label,
      status:
        activity.status === "running" ? "running" : activity.status === "error" ? "error" : "done",
      startedAt: Date.parse(activity.started_at),
      finishedAt: activity.ended_at ? Date.parse(activity.ended_at) : undefined,
    })),
  };
}

function path(id: string): string {
  if (!isNyxAgentConversationId(id)) throw new Error("Conversation not found.");
  return `${ROOT}/conversations/${id}`;
}

function storedError(code: string | null): string {
  if (code === "outcome_unknown") {
    return "The previous operation may have taken effect. Check its result before trying again.";
  }
  if (code === "credential_invalid") return "The assistant credential could not be accepted.";
  if (code === "model_not_configured") return "This assistant profile is unavailable.";
  if (code === "session_busy" || code === "capacity_exceeded") {
    return "The assistant is busy. Try again shortly.";
  }
  return "The assistant could not complete this turn.";
}

function active(row: NyxAgentConversation | undefined): boolean {
  return Boolean(row?.active_turn);
}

async function subscriptionDeadline<T>(
  operation: Promise<T>,
  milliseconds: number,
  controller: AbortController,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      operation,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          controller.abort();
          reject(new Error("The connection was interrupted. The assistant may still be working."));
        }, milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

/** Only a presentation cache. MongoDB owns transcripts, running turns and stop. */
export class NyxAgentTransport {
  private generation = 0;
  private owner = getAssistantIdentityUserId();
  private revision = 0;
  private mutationRevision = 0;
  private rowRevisions = new Map<string, number>();
  private listeners = new Set<() => void>();
  private index = new Map<string, NyxAgentConversation>();
  private histories = new Map<string, NyxAgentHistory>();
  private live = new Map<string, LiveTurn>();
  private requests = new Set<AbortController>();
  private draftModel = "nyxagent/chat";

  constructor() {
    subscribeAssistantIdentity(() => this.clear());
  }

  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  readonly getRevision = () => this.revision;

  private changed() {
    this.revision += 1;
    for (const listener of this.listeners) listener();
  }

  clear() {
    this.generation += 1;
    this.mutationRevision += 1;
    this.rowRevisions.clear();
    this.owner = getAssistantIdentityUserId();
    for (const request of this.requests) request.abort();
    this.requests.clear();
    this.live.clear();
    this.index.clear();
    this.histories.clear();
    this.draftModel = "nyxagent/chat";
    this.changed();
  }

  private identity() {
    if (this.owner !== getAssistantIdentityUserId()) this.clear();
    if (!this.owner) throw new Error("Sign in to use the assistant.");
    return this.generation;
  }

  private current(generation: number) {
    if (generation !== this.generation || this.owner !== getAssistantIdentityUserId()) {
      throw new DOMException("Identity changed", "AbortError");
    }
  }

  getConversations() {
    return [...this.index.values()].sort((a, b) =>
      b.last_message_at.localeCompare(a.last_message_at),
    );
  }

  getHistory(id?: string) {
    return id ? this.histories.get(id) : undefined;
  }

  getModel(id?: string) {
    return (id ? this.index.get(id)?.model : this.draftModel) ?? "nyxagent/chat";
  }

  setModel(model: string) {
    this.draftModel = model;
    this.changed();
  }

  getAccessMode(id?: string): NyxAgentAccessMode {
    if (id) return this.index.get(id)?.access_mode ?? "ask";
    const store = useAssistantDraftStore.getState();
    return store.ownerUserId === this.owner && store.nyxAgentAccessMode === "full" ? "full" : "ask";
  }

  async setAccessMode(id: string | undefined, accessMode: NyxAgentAccessMode) {
    const generation = this.identity();
    if (id) {
      const row = nyxAgentConversationSchema.parse(
        await assistantJson(`${path(id)}/access-mode`, {
          method: "PATCH",
          body: { access_mode: accessMode },
        }),
      );
      this.current(generation);
      if (row.id !== id) throw new Error("Assistant conversation mismatch.");
      this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
      this.mutationRevision += 1;
      this.index.set(id, row);
      const history = this.histories.get(id);
      if (history) this.histories.set(id, { ...history, conversation: row });
    }
    useAssistantDraftStore.getState().setNyxAgentAccessMode(this.owner!, accessMode);
    this.changed();
  }

  isRunning(id?: string) {
    return (
      this.live.has(id ?? "draft") ||
      active(id ? this.index.get(id) : undefined) ||
      active(this.getHistory(id)?.conversation)
    );
  }

  async models() {
    const generation = this.identity();
    const rows = nyxAgentModelsSchema.parse(await assistantJson(`${ROOT}/models`));
    this.current(generation);
    return rows;
  }

  async list() {
    const generation = this.identity();
    const mutationRevision = this.mutationRevision;
    let cursor: string | null = null;
    const seen = new Set<string>();
    const rows = new Map<string, NyxAgentConversation>();
    do {
      const query = cursor ? `&cursor=${encodeURIComponent(cursor)}` : "";
      const page = nyxAgentIndexSchema.parse(
        await assistantJson(`${ROOT}/conversations?limit=100${query}`),
      );
      this.current(generation);
      for (const row of page.conversations) rows.set(row.id, row);
      cursor = page.next_cursor;
      if (cursor && seen.has(cursor)) throw new Error("Invalid assistant pagination.");
      if (cursor) seen.add(cursor);
      if (seen.size > 1000) throw new Error("Assistant history is too large to load.");
    } while (cursor);
    if (mutationRevision !== this.mutationRevision) return this.getConversations();
    for (const [id, turn] of this.live) {
      if (id !== "draft") rows.set(id, turn.conversation);
    }
    this.index = rows;
    this.changed();
    return this.getConversations();
  }

  async history(id: string, beforeSeq?: number) {
    const generation = this.identity();
    const rowRevision = this.rowRevisions.get(id);
    const query = beforeSeq ? `&before_seq=${String(beforeSeq)}` : "";
    const page = nyxAgentHistorySchema.parse(await assistantJson(`${path(id)}?limit=100${query}`));
    this.current(generation);
    if (rowRevision !== this.rowRevisions.get(id)) {
      throw new DOMException("Conversation changed", "AbortError");
    }
    if (page.conversation.id !== id) throw new Error("Assistant conversation mismatch.");
    const existing = this.histories.get(id);
    // Preserve older pages during the running-turn poll and deduplicate by seq.
    const merged = new Map((existing?.messages ?? []).map((message) => [message.seq, message]));
    for (const message of page.messages) merged.set(message.seq, message);
    const before_seq = beforeSeq || !existing ? page.before_seq : existing.before_seq;
    const history = {
      ...page,
      before_seq,
      messages: [...merged.values()].sort((a, b) => a.seq - b.seq),
    };
    this.histories.set(id, history);
    this.index.set(id, page.conversation);
    this.changed();
    return history;
  }

  async rename(id: string, title: string) {
    const generation = this.identity();
    const row = nyxAgentConversationSchema.parse(
      await assistantJson(path(id), { method: "PATCH", body: { title } }),
    );
    this.current(generation);
    if (row.id !== id) throw new Error("Assistant conversation mismatch.");
    this.mutationRevision += 1;
    this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
    this.index.set(id, row);
    const history = this.histories.get(id);
    if (history) this.histories.set(id, { ...history, conversation: row });
    this.changed();
  }

  async delete(id: string) {
    const generation = this.identity();
    await assistantJson(path(id), { method: "DELETE" });
    this.current(generation);
    this.mutationRevision += 1;
    this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
    this.index.delete(id);
    this.histories.delete(id);
    this.changed();
  }

  async decide(id: string, acknowledgementId: string, decision: "allow" | "deny") {
    const generation = this.identity();
    const acknowledgement = nyxAgentAcknowledgementSchema.parse(
      await assistantJson(`${path(id)}/acknowledgements/${encodeURIComponent(acknowledgementId)}`, {
        method: "POST",
        body: { decision },
      }),
    );
    this.current(generation);
    if (acknowledgement.id !== acknowledgementId) {
      throw new Error("Assistant acknowledgement mismatch.");
    }
    // Fence a history poll that began before this decision committed.
    this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
    this.mutationRevision += 1;
    const history = this.histories.get(id);
    if (history) {
      const acknowledgements = history.acknowledgements.map((row) =>
        row.id === acknowledgementId ? acknowledgement : row,
      );
      const conversation = {
        ...history.conversation,
        pending_acknowledgements: acknowledgements.filter((row) => row.status === "pending").length,
      };
      this.histories.set(id, { ...history, conversation, acknowledgements });
      this.index.set(id, conversation);
      this.changed();
    }
    return acknowledgement;
  }

  async stop(id: string) {
    this.identity();
    await assistantJson(`${path(id)}/stop`, { method: "POST" });
    // The server settles/persists before we remove the live state. No resend.
  }

  session(id?: string): ChatSessionState {
    const history = this.getHistory(id);
    const live = this.live.get(id ?? "draft");
    const conversation = live?.conversation ?? history?.conversation ?? this.index.get(id ?? "");
    let messages: ChatMessage[] = (history?.messages ?? []).map((message) => {
      const failed = message.status === "failed" && message.error_code !== "cancelled";
      return {
        id: message.id,
        role: message.role,
        content: message.text,
        timestamp: Date.parse(message.created_at),
        turnId: message.turn_id,
        status: failed ? "error" : "complete",
        error: failed ? storedError(message.error_code) : undefined,
        ...toolCalls(message.activities),
      };
    });
    // The live turn's tool activity arrives through the polled history metadata,
    // which the server records at the MCP boundary; the upstream stream is text only.
    const liveTurnId = live?.state.activeTurn?.turnId ?? conversation?.active_turn?.turn_id;
    const polledTurn = history?.conversation.active_turn;
    const liveActivity =
      polledTurn && (!liveTurnId || polledTurn.turn_id === liveTurnId)
        ? toolCalls(polledTurn.activities)
        : {};
    if (live) {
      messages = live.state.messages.map((message) => {
        const streaming =
          message.role === "assistant" && message === live.state.messages.at(-1);
        return {
          id: message.id,
          role: message.role,
          content: message.blocks.map((block) => block.text).join("\n\n"),
          timestamp: Date.parse(message.created_at),
          status: streaming ? "streaming" : "complete",
          ...(streaming ? liveActivity : {}),
        };
      });
    }
    const running = Boolean(live || conversation?.active_turn);
    if (running && messages.at(-1)?.role !== "assistant") {
      messages.push({
        id: `${conversation?.active_turn?.turn_id ?? "pending"}-assistant`,
        role: "assistant",
        content: "",
        timestamp: Date.now(),
        status: "streaming",
        ...liveActivity,
      });
    }
    const resetAt = conversation?.context_reset_at;
    if (resetAt || live?.resetBeforeMessageId) {
      const timestamp = resetAt ? Date.parse(resetAt) : Date.now();
      const position = live?.resetBeforeMessageId
        ? messages.findIndex((message) => message.id === live.resetBeforeMessageId)
        : messages.findIndex((message) => message.timestamp > timestamp);
      messages.splice(position < 0 ? messages.length : position, 0, {
        id: `nyxagent-context-reset:${id ?? "draft"}`,
        role: "system",
        content: CONTEXT_RESET_NOTICE,
        timestamp,
        status: "complete",
      });
    }
    for (const acknowledgement of history?.acknowledgements ?? []) {
      const timestamp = Date.parse(acknowledgement.created_at);
      const position =
        acknowledgement.status === "pending"
          ? -1
          : messages.findIndex((message) => message.timestamp > timestamp);
      messages.splice(position < 0 ? messages.length : position, 0, {
        id: `nyxagent-acknowledgement:${acknowledgement.id}`,
        role: "system",
        content: acknowledgement.summary,
        timestamp,
        status: "complete",
      });
    }
    // Proxy approvals block the running tool call until decided; show them
    // at the tail while the turn waits.
    for (const approval of history?.approvals ?? []) {
      messages.push({
        id: `nyxagent-approval:${approval.id}`,
        role: "system",
        content: approval.summary,
        timestamp: Date.parse(approval.created_at),
        status: "complete",
      });
    }
    const tail = history?.messages.at(-1);
    return {
      clientId: id ?? "nyxagent-draft",
      conversationId: id,
      title: conversation?.title ?? "New chat",
      messages,
      expectedTurnCount: messages.filter((message) => message.role === "user").length,
      status: running
        ? "streaming"
        : tail?.error_code === "cancelled"
          ? "stopped"
          : tail?.status === "failed"
            ? "error"
            : "completed_text",
    };
  }

  async send(id: string | undefined, text: string, onAdopt: (id: string) => void) {
    const generation = this.identity();
    if (this.isRunning(id)) throw new Error("A turn is already active.");
    if (!text.trim() || [...text].length > 32768) {
      throw new Error("Message must contain 1 to 32768 characters.");
    }
    const controller = new AbortController();
    this.requests.add(controller);
    let key = id ?? "draft";
    const now = new Date().toISOString();
    const model = this.getModel(id);
    const userMessageId = crypto.randomUUID();
    const baseMessages = this.histories.get(key)?.messages ?? [];
    const turn: LiveTurn = {
      controller,
      conversation: this.index.get(key) ?? {
        id: key,
        title: [...text.trim()].slice(0, 40).join(""),
        model,
        created_at: now,
        last_message_at: now,
        access_mode: this.getAccessMode(id),
        message_count: 1,
        pending_acknowledgements: 0,
        active_turn: null,
        context_reset_at: null,
      },
      state: {
        lastCursor: 0,
        activeTurn: null,
        messages: [
          ...baseMessages.map((message) => ({
            id: message.id,
            role: message.role,
            schema_version: 1 as const,
            created_at: message.created_at,
            blocks: [{ type: "text" as const, block_id: message.id, text: message.text }],
          })),
          {
            id: userMessageId,
            role: "user",
            schema_version: 1,
            created_at: now,
            blocks: [{ type: "text", block_id: userMessageId, text }],
          },
        ],
      },
    };
    this.live.set(key, turn);
    this.changed();
    let adopted = Boolean(id);
    let terminal = false;
    try {
      const response = await subscriptionDeadline(
        assistantHttp(`${ROOT}/turns`, {
          method: "POST",
          body: {
            ...(id ? { conversation_id: id } : { model, access_mode: this.getAccessMode() }),
            text,
          },
          headers: { Accept: "text/event-stream" },
          signal: controller.signal,
        }),
        45_000,
        controller,
      );
      this.current(generation);
      if (!response.body) throw new Error("The assistant returned no stream.");
      const reader = response.body.getReader();
      const decoder = new TextDecoder("utf-8", { fatal: true });
      let buffer = "";
      let total = 0;
      try {
        for (;;) {
          const { done, value } = await subscriptionDeadline(reader.read(), 135_000, controller);
          this.current(generation);
          if (done) break;
          total += value.byteLength;
          if (total > 12 * 1024 * 1024) throw new Error("Assistant response exceeded its limit.");
          buffer += decoder.decode(value, { stream: true });
          const drained = drainDirectSseBuffer(buffer);
          buffer = drained.rest;
          for (const payload of drained.payloads) {
            const event = nyxAgentEventSchema.parse(JSON.parse(payload));
            if (event.cursor <= turn.state.lastCursor) continue;
            if (event.event === "turn.status") {
              if (adopted && event.conversation_id !== key) {
                throw new Error("Assistant conversation mismatch.");
              }
              if (!adopted) {
                this.live.delete(key);
                key = event.conversation_id;
                this.live.set(key, turn);
                adopted = true;
                turn.conversation = { ...turn.conversation, id: key };
                onAdopt(key);
              }
              turn.conversation = {
                ...turn.conversation,
                active_turn: { turn_id: event.turn_id, started_at: now, activities: [] },
              };
              this.index.set(key, turn.conversation);
            }
            if (!adopted) throw new Error("Assistant did not identify the conversation.");
            if (event.event === "turn.notice") {
              turn.resetBeforeMessageId = turn.state.messages.at(-1)?.id;
              this.changed();
              // Fetch the reset timestamp so a prior failed turn keeps its note
              // before this turn's user message; a rebind moves it before the reply.
              const refreshed = await this.history(key).catch(() => undefined);
              this.current(generation);
              if (refreshed?.conversation.context_reset_at) {
                turn.conversation = refreshed.conversation;
                const user = [...refreshed.messages]
                  .reverse()
                  .find((message) => message.role === "user");
                if (user) {
                  // Optimistic timestamps may predate a reset performed during admission.
                  turn.state = {
                    ...turn.state,
                    messages: turn.state.messages.map((message) =>
                      message.id === userMessageId
                        ? { ...message, created_at: user.created_at }
                        : message,
                    ),
                  };
                  const resetTime = Date.parse(turn.conversation.context_reset_at!);
                  if (Date.parse(user.created_at) > resetTime) {
                    delete turn.resetBeforeMessageId;
                  }
                }
              }
            }
            turn.state = applyDirectTurnEvent(turn.state, event);
            this.changed();
            if (event.event === "turn.completed") terminal = true;
          }
          if (terminal) break;
        }
      } finally {
        await reader.cancel().catch(() => undefined);
        reader.releaseLock();
      }
      if (!terminal) {
        throw new Error("The connection was interrupted. The assistant may still be working.");
      }
    } finally {
      this.requests.delete(controller);
      if (generation === this.generation) {
        if (adopted) await this.history(key).catch(() => undefined);
        this.current(generation);
        this.live.delete(key);
        this.changed();
        await this.list().catch(() => undefined);
      }
    }
  }
}

export const nyxAgentTransport = new NyxAgentTransport();
