import { formatCapacityWait } from "./capacity-wait";
import { ApiError } from "@/lib/api-client";
import { assistantHttp, assistantJson } from "@/lib/assistant/assistant-http";
import { billingError } from "@/lib/assistant/billing-error";
import {
  applyDirectTurnEvent,
  drainDirectSseBuffer,
  type DirectTurnState,
} from "@/lib/assistant/direct-transport";
import { getAssistantIdentityUserId, subscribeAssistantIdentity } from "@/lib/assistant/identity";
import { isNyxAgentConversationId } from "@/lib/assistant/conversation-ids";
import { isAdmittedSendError, markAdmitted } from "@/lib/assistant/send-admission";
import { withAssistantTrace } from "@/lib/assistant/request-error";
import type { ChatImage, ChatSessionState, ChatMessage } from "@/lib/assistant/chat-types";
import type { RuntimeToolCallInfo } from "@/lib/assistant/runtime-event-semantics";
import { deriveTurnStatus } from "@/lib/assistant/turn-status";
import {
  nyxAgentAcknowledgementSchema,
  nyxAgentSteeringSchema,
  nyxAgentCapabilitiesSchema,
  nyxAgentConversationSchema,
  nyxAgentHistorySchema,
  nyxAgentIndexSchema,
  nyxAgentEventSchema,
  nyxAgentRecoverySchema,
  type NyxAgentConversation,
  type NyxAgentConversationAgent,
  type NyxAgentHistory,
  type NyxAgentTurnActivity,
  type NyxAgentAttachment,
} from "@/schemas/assistant-nyxagent";

const ROOT = "/assistant/nyxagent";
export const CONTEXT_RESET_NOTICE =
  "The last turn failed, so the assistant continues in a fresh context with a recap of its completed actions, failures and attachments. Rerun only what did not complete.";

type NyxAgentMessage = NyxAgentHistory["messages"][number];
type LiveMessage = DirectTurnState["messages"][number];

interface LiveTurn {
  state: DirectTurnState;
  conversation: NyxAgentConversation;
  controller: AbortController;
  baseMessageIds: ReadonlySet<string>;
  resetBeforeMessageId?: string;
  retrying?: boolean;
  settled?: NyxAgentMessage;
  replyCursor?: number;
}

function restoreReply(turn: LiveTurn, activeTurn: NyxAgentConversation["active_turn"]) {
  const reply = activeTurn?.reply;
  if (!reply || reply.cursor <= (turn.replyCursor ?? 0)) return;
  const currentTurn = turn.state.activeTurn?.turnId ?? turn.conversation.active_turn?.turn_id;
  if (currentTurn && currentTurn !== activeTurn.turn_id) return;
  const message: LiveMessage = {
    id: reply.message_id, role: "assistant", schema_version: 1,
    created_at: activeTurn.started_at,
    blocks: [{ type: "text", block_id: reply.block_id, text: reply.text }],
  };
  const found = turn.state.messages.some((row) => row.id === reply.message_id);
  turn.state = {
    ...turn.state,
    lastCursor: Math.max(reply.cursor, turn.state.lastCursor),
    activeTurn: turn.state.activeTurn ?? { turnId: activeTurn.turn_id, status: "running", error: null },
    messages: found
      ? turn.state.messages.map((row) => row.id === reply.message_id ? message : row)
      : [...turn.state.messages, message],
  };
  turn.replyCursor = reply.cursor;
}

function toolCalls(
  activities: readonly NyxAgentTurnActivity[] | undefined,
  conversationId?: string,
): { toolCalls?: RuntimeToolCallInfo[] } {
  if (!activities?.length) return {};
  return {
    toolCalls: activities.map((activity) => ({
      id: activity.id,
      ...(activity.machine ? { machine: activity.machine, conversationId } : {}),
      name: activity.label,
      status:
        activity.status === "running" ? "running" : activity.status === "error" ? "error" : "done",
      startedAt: Date.parse(activity.started_at),
      finishedAt: activity.ended_at ? Date.parse(activity.ended_at) : undefined,
      failure: activity.failure,
    })),
  };
}

function images(
  conversationId: string | undefined,
  attachments: readonly NyxAgentAttachment[] | undefined,
): { images?: ChatImage[] } {
  if (!conversationId || !attachments?.length) return {};
  return {
    images: attachments.map((attachment) => ({
      id: attachment.id,
      expired: attachment.expired,
      endpoint: `${path(conversationId)}/attachments/${encodeURIComponent(attachment.id)}`,
      contentType: attachment.content_type,
      label: attachment.label,
      origin: attachment.origin || undefined,
      ...(attachment.image_input ? { imageInput: attachment.image_input } : {}),
    })),
  };
}

function path(id: string): string {
  if (!isNyxAgentConversationId(id)) throw new Error("Conversation not found.");
  return `${ROOT}/conversations/${id}`;
}

export function storedError(code: string | null, capacityWaitMs?: number | null): string {
  const billing = billingError(code);
  if (billing) return billing.message;
  if (code === "tool_budget_exhausted" || code === "turn_timeout") return "This task reached its automatic continuation limit. Its context is saved; resume to continue, or adjust the limit in NyxBot settings.";
  if (code === "continuation_no_progress") return "Paused because the task repeated without progress. Its context is saved; give it new guidance.";
  if (code === "idle_timeout") return "The assistant stopped responding. Check what it already did before asking it to continue.";
  if (code === "outcome_unknown") {
    return "The previous operation may have taken effect. Check its result before trying again.";
  }
  if (code === "credential_invalid") return "The assistant credential could not be accepted.";
  if (code === "model_not_configured") return "This assistant profile is unavailable. Choose another model in the model selector.";
  if (code === "model_unavailable") return "The selected model is unavailable. Choose another model in the model selector.";
  if (code === "model_interrupted") return "The model's response was interrupted. Some actions may already have completed — check them before retrying.";
  if (code === "session_busy") return "The assistant session is busy. Try again shortly.";
  if (code === "capacity_exceeded") {
    const waited = capacityWaitMs != null && capacityWaitMs > 0
      ? ` (waited ${formatCapacityWait(capacityWaitMs)})` : "";
    return `NyxBot is at capacity right now${waited}. Try again shortly.`;
  }
  if (code === "turn_lost") return "This turn was interrupted; check which actions completed before retrying.";
  if (code === "internal_error") return "NyxBot hit an internal error on this turn. Check which actions completed before retrying.";
  if (code === "assistant_unavailable") return "The assistant could not complete this turn. Try again.";
  if (code === "first_byte_timeout") return "The assistant did not start responding in time. Try again.";
  if (code === "invalid_stream") return "The assistant stream ended unexpectedly. Try again.";
  if (code === "output_too_large") return "The assistant reached its response limit.";
  if (code === "nyxid_error") return "NyxID could not authorize the assistant operation.";
  if (code === "stale_response") return "The assistant context changed. Start a new turn.";
  if (code === "context_unavailable") return "The assistant context could not be restored. Try a new turn.";
  return "The assistant could not complete this turn.";
}

export function contextResetCopy(reason: string | null | undefined): string {
  switch (reason) {
    case "turn_failed": return CONTEXT_RESET_NOTICE;
    case "instructions_changed": return "Instructions changed; the assistant context was refreshed.";
    case "model_changed": return "The model changed; the assistant context was refreshed.";
    case "session_lost": return "The assistant session expired; a fresh context will be used.";
    case "audience_changed": return "The conversation audience changed; the assistant context was refreshed.";
    case "guest_turn": return "This guest turn uses a separate assistant context.";
    case "stopped": return "The turn was stopped, so the assistant continues in a fresh context with a recap. Check what the stopped turn finished before rerunning it.";
    case "credential_replaced": return "The assistant's key was replaced, so it continues in a fresh context with a recap of this chat.";
    case "user_reset": return "You reset the working context.";
    default: return CONTEXT_RESET_NOTICE;
  }
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
  private interrupted = new Set<string>();
  private owner = getAssistantIdentityUserId();
  private revision = 0;
  private mutationRevision = 0;
  private rowRevisions = new Map<string, number>();
  private pendingTitles = new Map<string, string>();
  private listeners = new Set<() => void>();
  private index = new Map<string, NyxAgentConversation>();
  private histories = new Map<string, NyxAgentHistory>();
  private historyRequests = new Map<string, number>();
  private historyApplied = new Map<string, number>();
  private consumedRetries = new Map<string, Set<string>>();
  private acceptedTurns = new Map<string, string>();
  private reviewCards = new Map<string, string>();
  private live = new Map<string, LiveTurn>();
  private requests = new Set<AbortController>();
  private draftModel = "nyxagent/chat";
  // Views reused while their source is unchanged, so a streamed token or a
  // history refresh re-renders only the rows that actually changed.
  private settledViews = new WeakMap<NyxAgentMessage, ChatMessage>();
  private liveViews = new WeakMap<LiveMessage, { id?: string; settled?: NyxAgentMessage; view: ChatMessage }>();
  private views = new Map<string, Map<string, ChatMessage>>();

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
    this.interrupted.clear();
    this.mutationRevision += 1;
    this.rowRevisions.clear();
    this.pendingTitles.clear();
    this.owner = getAssistantIdentityUserId();
    for (const request of this.requests) request.abort();
    this.requests.clear();
    this.live.clear();
    this.index.clear();
    this.histories.clear();
    this.historyRequests.clear();
    this.historyApplied.clear();
    this.consumedRetries.clear();
    this.acceptedTurns.clear();
    this.reviewCards.clear();
    this.views.clear();
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

  /** Known threads, newest first; only `agentId`'s when given. */
  getConversations(agentId?: string) {
    return [...this.index.values()]
      .filter((row) => agentId === undefined || row.agent?.id === agentId)
      .sort((a, b) => b.last_message_at.localeCompare(a.last_message_at));
  }

  /** The cached row for any thread, whichever agent it belongs to. */
  getConversation(id?: string): NyxAgentConversation | undefined {
    if (!id) return undefined;
    return this.live.get(id)?.conversation ?? this.histories.get(id)?.conversation ?? this.index.get(id);
  }

  /** Drop every cached thread of an agent that was deleted permanently. */
  forgetAgent(agentId: string) {
    this.mutationRevision += 1;
    for (const row of this.getConversations(agentId)) {
      this.rowRevisions.set(row.id, (this.rowRevisions.get(row.id) ?? 0) + 1);
      this.index.delete(row.id);
      this.histories.delete(row.id);
      this.consumedRetries.delete(row.id);
      this.acceptedTurns.delete(row.id);
      this.reviewCards.delete(row.id);
      this.views.delete(row.id);
    }
    this.changed();
  }

  getHistory(id?: string) {
    return id ? this.histories.get(id) : undefined;
  }

  getModel(id?: string) {
    return (id ? this.getConversation(id)?.model : this.draftModel) ?? "nyxagent/chat";
  }

  setModel(model: string) {
    this.draftModel = model;
    this.changed();
  }

  isRunning(id?: string) {
    return (
      this.live.has(id ?? "draft") ||
      active(id ? this.index.get(id) : undefined) ||
      active(this.getHistory(id)?.conversation)
    );
  }

  /** Every thread, or only one agent's threads when `agentId` is given. */
  async list(agentId?: string) {
    const generation = this.identity();
    const mutationRevision = this.mutationRevision;
    let cursor: string | null = null;
    const seen = new Set<string>();
    const rows = new Map<string, NyxAgentConversation>();
    const scope = agentId ? `&agent_id=${encodeURIComponent(agentId)}` : "";
    do {
      const query = cursor ? `&cursor=${encodeURIComponent(cursor)}` : "";
      const page = nyxAgentIndexSchema.parse(
        await assistantJson(`${ROOT}/conversations?limit=100${scope}${query}`),
      );
      this.current(generation);
      for (const row of page.conversations) {
        const title = this.pendingTitles.get(row.id);
        rows.set(row.id, title === undefined ? row : { ...row, title, title_source: "user" });
      }
      cursor = page.next_cursor;
      if (cursor && seen.has(cursor)) throw new Error("Invalid assistant pagination.");
      if (cursor) seen.add(cursor);
      if (seen.size > 1000) throw new Error("Assistant history is too large to load.");
    } while (cursor);
    if (mutationRevision !== this.mutationRevision) return this.getConversations(agentId);
    for (const [id, turn] of this.live) {
      if (id !== "draft" && (!agentId || turn.conversation.agent?.id === agentId)) {
        const current = rows.get(id);
        if (current) {
          turn.conversation = { ...turn.conversation, title: current.title, title_source: current.title_source };
        }
        rows.set(id, turn.conversation);
      }
    }
    if (agentId) {
      // Replace only this agent's threads; other agents' rows stay as they are.
      for (const row of this.getConversations(agentId)) this.index.delete(row.id);
      for (const [id, row] of rows) this.index.set(id, row);
    } else {
      this.index = rows;
    }
    this.changed();
    return this.getConversations(agentId);
  }

  async history(id: string, beforeSeq?: number, reviewCard?: string | null) {
    const generation = this.identity();
    const request = (this.historyRequests.get(id) ?? 0) + 1;
    this.historyRequests.set(id, request);
    // Internal refreshes and pagination retain the linked card. The route
    // passes null explicitly when the owner leaves that review link.
    if (reviewCard !== undefined) {
      if (reviewCard === null) this.reviewCards.delete(id);
      else this.reviewCards.set(id, reviewCard);
    }
    const card = this.reviewCards.get(id);
    const rowRevision = this.rowRevisions.get(id);
    const query = beforeSeq ? `&before_seq=${String(beforeSeq)}` : "";
    const endpoint = `${path(id)}?limit=100${query}`;
    const controller = new AbortController();
    this.requests.add(controller);
    const fetchHistory = (url: string) => subscriptionDeadline(
      assistantJson(url, { signal: controller.signal }), 15_000, controller,
    );
    let response: unknown;
    try {
      response = await fetchHistory(card ? `${endpoint}&card=${encodeURIComponent(card)}` : endpoint);
    } catch (error) {
      // Older replicas reject the additive query field; ordinary history still works.
      if (!card || !(error instanceof ApiError) || error.status !== 400) throw error;
      response = await fetchHistory(endpoint);
    } finally {
      this.requests.delete(controller);
    }
    const page = nyxAgentHistorySchema.parse(response);
    this.current(generation);
    if (rowRevision !== this.rowRevisions.get(id) || card !== this.reviewCards.get(id)) {
      throw new DOMException("Conversation changed", "AbortError");
    }
    if (page.conversation.id !== id) throw new Error("Assistant conversation mismatch.");
    const existing = this.histories.get(id);
    if (request < (this.historyApplied.get(id) ?? 0) && existing) {
      if (!beforeSeq) return existing;
      page.conversation = existing.conversation;
    } else {
      this.historyApplied.set(id, request);
    }
    const optimisticTitle = this.pendingTitles.get(id);
    if (optimisticTitle !== undefined) {
      page.conversation = { ...page.conversation, title: optimisticTitle, title_source: "user" };
    }
    if (page.conversation.active_turn && this.acceptedTurns.get(id) === page.conversation.active_turn.turn_id) {
      page.conversation.active_turn = { ...page.conversation.active_turn, waiting_for_capacity: false, capacity_wait_note_origin: null };
    }
    const live = this.live.get(id);
    const activeTurn = page.conversation.active_turn;
    if (activeTurn) {
      const previous = existing?.conversation.active_turn;
      if (previous?.turn_id === activeTurn.turn_id && (previous.reply?.cursor ?? 0) > (activeTurn.reply?.cursor ?? 0)) {
        activeTurn.reply = previous.reply;
      }
    }
    if (live) {
      restoreReply(live, page.conversation.active_turn);
      const turnId = live.state.activeTurn?.turnId ?? live.conversation.active_turn?.turn_id;
      const settled = page.messages.find((message) => message.role === "assistant" && message.turn_id === turnId && !live.baseMessageIds.has(message.id));
      if (settled) {
        live.settled = settled;
        live.controller.abort();
      }
      live.conversation = {
        ...live.conversation,
        title: page.conversation.title,
        title_source: page.conversation.title_source,
        ...(activeTurn && (!turnId || activeTurn.turn_id === turnId) ? { active_turn: activeTurn } : {}),
      };
    }
    // Preserve older pages during the running-turn poll and deduplicate by seq.
    // An unchanged message keeps its object so its rendered row is reused.
    const merged = new Map((existing?.messages ?? []).map((message) => [message.seq, message]));
    for (const message of page.messages) {
      const previous = merged.get(message.seq);
      const received = this.consumedRetries.get(id)?.has(message.id) && message.recovery
        ? { ...message, recovery: { ...message.recovery, status: "review" as const, reason: "already_retried" } }
        : message;
      merged.set(
        message.seq,
        previous && JSON.stringify(previous) === JSON.stringify(received) ? previous : received,
      );
    }
    const before_seq = beforeSeq || !existing ? page.before_seq : existing.before_seq;
    const history = {
      ...page,
      before_seq,
      messages: [...merged.values()].sort((a, b) => a.seq - b.seq),
    };
    this.histories.set(id, history);
    if (!history.conversation.active_turn) this.interrupted.delete(id);
    this.index.set(id, page.conversation);
    this.changed();
    return history;
  }

  private setTitle(id: string, title: string, source: NyxAgentConversation["title_source"]) {
    const row = this.index.get(id);
    if (row) this.index.set(id, { ...row, title, title_source: source });
    const history = this.histories.get(id);
    if (history) this.histories.set(id, { ...history, conversation: { ...history.conversation, title, title_source: source } });
    const live = this.live.get(id);
    if (live) live.conversation = { ...live.conversation, title, title_source: source };
    this.changed();
  }

  async capabilities(id: string) {
    const generation = this.identity();
    const result = nyxAgentCapabilitiesSchema.parse(await assistantJson(`${path(id)}/capabilities`));
    this.current(generation);
    return result;
  }

  async steer(id: string, text: string, turnId: string, clientRequestId: string) {
    const generation = this.identity();
    try {
      const result = nyxAgentSteeringSchema.parse(await assistantJson(`${path(id)}/steer`, {
        method: "POST", body: { text, turn_id: turnId, clientRequestId },
      }));
      this.current(generation);
      return result;
    } finally {
      // Receipt and response metadata arrive through the same history path as
      // another tab's guidance. A refresh failure cannot change acceptance.
      if (generation === this.generation) await this.history(id).catch(() => undefined);
    }
  }

  async rename(id: string, title: string) {
    const generation = this.identity();
    const before = this.getConversation(id);
    if (this.pendingTitles.has(id)) throw new Error("A rename is already in progress.");
    this.pendingTitles.set(id, title.trim());
    this.mutationRevision += 1;
    this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
    this.setTitle(id, title.trim(), "user");
    try {
      const row = nyxAgentConversationSchema.parse(
        await assistantJson(path(id), { method: "PATCH", body: { title } }),
      );
      this.current(generation);
      if (row.id !== id) throw new Error("Assistant conversation mismatch.");
      this.setTitle(id, row.title, row.title_source ?? "user");
    } catch (error) {
      if (generation === this.generation && before) this.setTitle(id, before.title, before.title_source);
      throw error;
    } finally {
      if (generation === this.generation) {
        this.pendingTitles.delete(id);
        this.mutationRevision += 1;
        this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
      }
    }
  }

  async delete(id: string) {
    const generation = this.identity();
    await assistantJson(path(id), { method: "DELETE" });
    this.current(generation);
    this.mutationRevision += 1;
    this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
    this.index.delete(id);
    this.histories.delete(id);
    this.consumedRetries.delete(id);
    this.acceptedTurns.delete(id);
    this.reviewCards.delete(id);
    this.views.delete(id);
    this.changed();
  }

  async decide(id: string, acknowledgementId: string, decision: "allow" | "deny", acceptReducedScope = false) {
    const generation = this.identity();
    const acknowledgement = nyxAgentAcknowledgementSchema.parse(
      await assistantJson(`${path(id)}/acknowledgements/${encodeURIComponent(acknowledgementId)}`, {
        method: "POST",
        body: { decision, ...(acceptReducedScope ? { accept_reduced_scope: true } : {}) },
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

  /**
   * The turn this page currently sees running, live or polled. Loaded
   * history is authoritative, including its explicit idle state; the index is
   * only a fallback before history loads, since a slower index page can land
   * after a newer history snapshot.
   */
  getActiveTurnId(id?: string): string | undefined {
    const key = id ?? "draft";
    const live = this.live.get(key)?.conversation.active_turn?.turn_id;
    if (live) return live;
    const history = this.histories.get(key);
    if (history) return history.conversation.active_turn?.turn_id ?? undefined;
    return this.index.get(key)?.active_turn?.turn_id ?? undefined;
  }

  continuations(id?: string): number {
    return this.live.get(id ?? "draft")?.state.activeTurn?.continuations ?? this.getHistory(id)?.conversation.active_turn?.continuations ?? 0;
  }

  session(id?: string): ChatSessionState {
    const history = this.getHistory(id);
    const observedLive = this.live.get(id ?? "draft");
    const live = observedLive?.settled ? undefined : observedLive;
    const conversation = live?.conversation ?? history?.conversation ?? this.index.get(id ?? "");
    let messages: ChatMessage[] = (history?.messages ?? []).map((message) => {
      const cached = this.settledViews.get(message);
      if (cached) return cached;
      const failed = message.status === "failed" && message.error_code !== "cancelled";
      const view: ChatMessage = {
        recovery: message.recovery,
        id: message.id,
        feedback: message.feedback ?? undefined,
        role: message.role,
        content: message.text,
        timestamp: Date.parse(message.created_at),
        turnId: message.turn_id,
        status: failed ? "error" : message.error_code === "cancelled" ? "stopped" : "complete",
        error: failed ? storedError(message.error_code, message.error_detail?.capacity_wait_ms) : undefined,
        channelDelivery: message.turn_metadata?.channel_delivery,
        turnMetadata: message.turn_metadata ? { attempts: message.turn_metadata.attempts, capacityWaitMs: message.turn_metadata.capacity_wait_ms } : undefined,
        errorDetail: message.error_detail ? { attempts: message.error_detail.attempts, capacityWaitMs: message.error_detail.capacity_wait_ms, upstreamCode: message.upstream_error_code, stage: message.error_detail.stage, httpStatus: message.error_detail.http_status, upstreamRequestId: message.error_detail.upstream_request_id, responseId: message.error_detail.response_id, continuations: message.error_detail.continuations, turnId: message.error_detail.turn_id, occurredAt: message.error_detail.occurred_at } : undefined,
        errorCode: message.error_code,
        ...toolCalls(message.activities, id),
        ...images(id, message.attachments),
        via: message.via ?? (message.platform_origin ? "telegram" : null),
        activityTotal: message.activity_total,
        steering: message.steering,
        continuation_of: message.continuation_of,
      };
      this.settledViews.set(message, view);
      return view;
    });
    // The live turn's tool activity arrives through the polled history metadata,
    // which the server records at the MCP boundary; the upstream stream is text only.
    const liveTurnId = live?.state.activeTurn?.turnId ?? conversation?.active_turn?.turn_id;
    const polledTurn = history?.conversation.active_turn;
    const liveActivity =
      polledTurn && (!liveTurnId || polledTurn.turn_id === liveTurnId)
        ? { ...toolCalls(polledTurn.activities, id), ...images(id, polledTurn.attachments), activityTotal: polledTurn.activity_total }
        : {};
    const settledImages = new Map(
      (history?.messages ?? []).map((message) => [message.id, images(id, message.attachments)]),
    );
    const turnStatus = conversation
      ? deriveTurnStatus({
          activeTurn: conversation.active_turn,
          live: Boolean(live),
          pendingApproval: Boolean(
            (history?.acknowledgements ?? []).some((row) => row.status === "pending")
            || (history?.approvals ?? []).length,
          ),
          retrying: Boolean(live?.retrying),
          interrupted: this.interrupted.has(id ?? "draft"),
        })
      : null;
    if (live) {
      const settledMessages = new Map(
        (history?.messages ?? []).map((message) => [message.id, message]),
      );
      messages = live.state.messages.map((message) => {
        const streaming =
          message.role === "assistant" && message === live.state.messages.at(-1);
        const settled = settledMessages.get(message.id);
        const settledView = settled && this.settledViews.get(settled);
        if (!streaming && settledView) return settledView;
        const cached = streaming ? undefined : this.liveViews.get(message);
        if (cached && cached.id === id && cached.settled === settled) return cached.view;
        const view: ChatMessage = {
          recovery: settled?.recovery,
          id: message.id,
          role: settled?.role ?? message.role,
          feedback: settled?.feedback ?? undefined,
          errorCode: settled?.error_code ?? (streaming ? live.state.activeTurn?.error?.code : undefined),
          error: streaming && live.state.activeTurn?.error ? storedError(live.state.activeTurn.error.code, live.state.activeTurn.error.capacity_wait_ms) : undefined,
          channelDelivery: settled?.turn_metadata?.channel_delivery,
          turnMetadata: streaming ? live.state.activeTurn?.turnMetadata : settled?.turn_metadata
            ? { attempts: settled.turn_metadata.attempts, capacityWaitMs: settled.turn_metadata.capacity_wait_ms } : undefined,
          errorDetail: streaming && live.state.activeTurn?.error?.stage ? {
            attempts: live.state.activeTurn.error.attempts, capacityWaitMs: live.state.activeTurn.error.capacity_wait_ms,
            stage: live.state.activeTurn.error.stage,
            upstreamCode: live.state.activeTurn.error.upstream_error_code,
            httpStatus: live.state.activeTurn.error.http_status,
            upstreamRequestId: live.state.activeTurn.error.upstream_request_id,
            responseId: live.state.activeTurn.error.response_id,
            continuations: live.state.activeTurn.continuations ?? 0,
            turnId: live.state.activeTurn.turnId ?? "",
          } : undefined,
          steering: settled?.steering,
          continuation_of: settled?.continuation_of ?? message.continuation_of,
          via: settled?.via ?? (settled?.platform_origin ? "telegram" : null),
          turnId: settled?.turn_id ?? (streaming ? live.state.activeTurn?.turnId ?? conversation?.active_turn?.turn_id : undefined),
          content: message.blocks.map((block) => block.text).join("\n\n"),
          timestamp: Date.parse(message.created_at),
          status: streaming ? (live.state.activeTurn?.status === "failed" ? "error" : "streaming") : "complete",
          ...(streaming ? { turnStatus } : {}),
          ...(message.attachments?.length
            ? images(id, message.attachments)
            : streaming
              ? liveActivity
              : settledImages.get(message.id)),
        };
        if (!streaming) this.liveViews.set(message, { id, settled, view });
        return view;
      });
    }
    if (live) {
      for (const message of history?.messages ?? []) {
        if (!message.steering || messages.some((m) => m.id === message.id)) continue;
        const position = messages.findIndex((m) => m.status === "streaming");
        const view = this.settledViews.get(message);
        if (view) {
          messages.splice(position < 0 ? messages.length : position, 0, view);
        }
      }
    }
    const running = Boolean(live || conversation?.active_turn);
    const liveSettled = live?.state.activeTurn && ["completed", "failed", "cancelled"].includes(live.state.activeTurn.status);
    if (running && !liveSettled && messages.at(-1)?.status !== "streaming") {
      messages.push({
        id: conversation?.active_turn?.reply?.message_id ?? `${conversation?.active_turn?.turn_id ?? "pending"}-assistant`,
        turnId: conversation?.active_turn?.turn_id ?? live?.state.activeTurn?.turnId,
        role: "assistant",
        content: conversation?.active_turn?.reply?.text ?? "",
        timestamp: conversation?.active_turn ? Date.parse(conversation.active_turn.started_at) : Date.now(),
        status: "streaming",
        ...liveActivity,
        turnStatus,
      });
    }
    if ((conversation?.active_turn?.waiting_for_capacity || conversation?.active_turn?.capacity_wait_note_origin === "nyxid_system") && this.acceptedTurns.get(conversation.id) !== conversation.active_turn?.turn_id) {
      messages = messages.map((message) => message.status === "streaming" ? { ...message, waitingForCapacity: conversation.active_turn?.waiting_for_capacity, systemNote: conversation.active_turn?.capacity_wait_note_origin === "nyxid_system" ? "capacity_wait" : undefined, capacityWaitStartedAt: conversation.active_turn?.engine_request_started_at ?? conversation.active_turn?.capacity_wait_started_at, capacityWaitDeadline: conversation.active_turn?.capacity_wait_deadline } : message);
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
        content: contextResetCopy(conversation?.context_reset_reason),
        timestamp,
        status: "complete",
      });
    }
    // Build a linear, exact-turn join. The receipt keeps its own virtual row
    // and stable card ID, so deep links and offscreen focusing keep working.
    const replies = new Map(messages.filter((m) => m.role === "assistant" && m.turnId)
      .map((m) => [m.turnId!, m.id]));
    const footers = new Map<string, ChatMessage[]>();
    for (const acknowledgement of history?.acknowledgements ?? []) {
      const timestamp = Date.parse(acknowledgement.created_at);
      const receipt: ChatMessage = {
        id: `nyxagent-acknowledgement:${acknowledgement.id}`,
        role: "system", content: acknowledgement.summary, timestamp, status: "complete",
      };
      const reply = acknowledgement.creation_turn_id && replies.get(acknowledgement.creation_turn_id);
      if (acknowledgement.status !== "pending" && reply) {
        const rows = footers.get(reply) ?? [];
        rows.push(receipt);
        footers.set(reply, rows);
      } else {
        // Legacy/unloaded anchors stay standalone; never infer a turn by time.
        const position = acknowledgement.status === "pending" ? -1
          : messages.findIndex((message) => message.timestamp > timestamp);
        messages.splice(position < 0 ? messages.length : position, 0, receipt);
      }
    }
    if (footers.size) messages = messages.flatMap((message) => [message, ...(footers.get(message.id) ?? [])]);
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
    // Rows rebuilt above with identical fields keep their previous object.
    const previousViews = this.views.get(id ?? "draft");
    const views = new Map<string, ChatMessage>();
    messages = messages.map((message) => {
      const previous = previousViews?.get(message.id);
      const view = previous && sameFields(previous, message) ? previous : message;
      views.set(message.id, view);
      return view;
    });
    this.views.set(id ?? "draft", views);
    const tail = history?.messages.at(-1);
    return {
      clientId: id ?? "nyxagent-draft",
      conversationId: id,
      title: conversation?.title ?? "New chat",
      messages,
      expectedTurnCount: messages.filter((message) => message.role === "user" && !message.steering).length,
      status: running
        ? "streaming"
        : tail?.error_code === "cancelled"
          ? "stopped"
          : tail?.status === "failed"
            ? "error"
            : "completed_text",
    };
  }

  async send(
    id: string | undefined,
    text: string,
    onAdopt: (id: string) => void,
    onTurnFailed?: (conversationId: string, turnId: string, code: string) => void,
    /** New threads only: the agent to talk to (the server defaults to NyxBot). */
    options: {
      readonly agent?: NyxAgentConversationAgent;
      readonly model?: string;
      readonly attachmentIds?: string[];
      readonly attachments?: readonly NyxAgentAttachment[];
      readonly adoptExisting?: boolean;
      readonly retryOf?: string;
      readonly acknowledgementId?: string;
    } = {},
  ) {
    const generation = this.identity();
    if (this.isRunning(id)) throw new Error("A turn is already active.");
    if ((!text.trim() && !options.attachmentIds?.length && !options.attachments?.length && !options.retryOf) || [...text].length > 32768) {
      throw new Error("Message must contain 1 to 32768 characters.");
    }
    const controller = new AbortController();
    this.requests.add(controller);
    let key = id ?? "draft";
    const now = new Date().toISOString();
    const model = (!id ? options.model : undefined) ?? this.getModel(id);
    const userMessageId = crypto.randomUUID();
    const baseMessages = this.histories.get(key)?.messages ?? [];
    const turn: LiveTurn = {
      baseMessageIds: new Set(baseMessages.map((message) => message.id)),
      controller,
      conversation: this.index.get(key) ??
        this.histories.get(key)?.conversation ?? {
          id: key,
          title: provisionalTitle(text),
          title_source: "provisional",
          model,
          created_at: now,
          last_message_at: now,
          message_count: 1,
          pending_acknowledgements: 0,
          active_turn: null,
          context_reset_at: null,
          role: options.agent?.kind === "specialist" ? "subagent" : "orchestrator",
          agent: options.agent ?? null,
          pending_events: 0,
          channel: null,
        },
      state: {
        lastCursor: 0,
        activeTurn: null,
        messages: [
          ...baseMessages.map((message) => ({
            id: message.id,
            // The live stream only distinguishes the reply; session() restores
            // orchestrator and event roles from the settled history.
            role: message.role === "assistant" ? ("assistant" as const) : ("user" as const),
            schema_version: 1 as const,
            created_at: message.created_at,
            continuation_of: message.continuation_of,
            blocks: [{ type: "text" as const, block_id: message.id, text: message.text }],
          })),
          {
            id: userMessageId,
            continuation_of: options.acknowledgementId,
            role: "user",
            schema_version: 1,
            created_at: now,
            blocks: [{ type: "text", block_id: userMessageId, text }],
            attachments: options.attachments,
          },
        ],
      },
    };
    this.interrupted.delete(key);
    this.live.set(key, turn);
    this.changed();
    let adopted = Boolean(id) && !options.adoptExisting;
    let terminal = false;
    let responded = false;
    let traceId: string | null = null;
    try {
      const response = await subscriptionDeadline(
        assistantHttp(`${ROOT}/turns`, {
          method: "POST",
          body: {
            ...(id
              ? { conversation_id: id }
              : { model, ...(options.agent ? { agent_id: options.agent.id } : {}) }),
            text,
            ...(options.retryOf ? { retry_of: options.retryOf } : {}),
            ...(options.acknowledgementId ? { acknowledgement_id: options.acknowledgementId } : {}),
            ...(options.attachmentIds?.length ? { attachment_ids: options.attachmentIds } : {}),
          },
          headers: { Accept: "text/event-stream" },
          signal: controller.signal,
          creditsDenial: {
            key: `assistant:nyxagent:${key}:${userMessageId}`,
            payer: "self",
          },
        }),
        45_000,
        controller,
      );
      responded = true;
      traceId = response.headers.get("X-NyxID-Trace-Id");
      this.current(generation);
      const responseId = response.headers.get("X-NyxID-Conversation-Id");
      if (responseId) {
        if (!isNyxAgentConversationId(responseId) || (adopted && responseId !== key)) {
          throw new Error("Assistant conversation mismatch.");
        }
        if (!adopted) {
          this.live.delete(key);
          key = responseId;
          turn.conversation = { ...turn.conversation, id: key };
          this.live.set(key, turn);
          adopted = true;
          onAdopt(key);
          void this.history(key).catch(() => undefined);
        }
      }
      if (!response.body) throw new Error("The assistant returned no stream.");
      const reader = response.body.getReader();
      const cancel = () => void reader.cancel().catch(() => undefined);
      controller.signal.addEventListener("abort", cancel, { once: true });
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
              if (event.engine_accepted) this.acceptedTurns.set(key, event.turn_id);
              const accepted = this.acceptedTurns.get(key) === event.turn_id;
              const previous = turn.conversation.active_turn?.turn_id === event.turn_id
                ? turn.conversation.active_turn : undefined;
              turn.conversation = {
                ...turn.conversation,
                active_turn: {
                  turn_id: event.turn_id,
                  started_at: event.started_at ?? previous?.started_at ?? now,
                  activities: previous?.activities ?? [],
                  attachments: previous?.attachments ?? [],
                  waiting_for_capacity: !accepted && (event.waiting_for_capacity ?? false),
                  capacity_wait_note_origin: !accepted && event.status === "waiting" ? event.capacity_wait_note_origin ?? previous?.capacity_wait_note_origin : null,
                  engine_request_started_at: event.engine_request_started_at ?? previous?.engine_request_started_at,
                  capacity_wait_started_at: event.capacity_wait_started_at ?? previous?.capacity_wait_started_at,
                  capacity_wait_deadline: event.capacity_wait_deadline ?? previous?.capacity_wait_deadline,
                },
              };
              this.index.set(key, turn.conversation);
            }
            if (!adopted) throw new Error("Assistant did not identify the conversation.");
            if (event.event === "turn.notice" && event.code === "context_reset") {
              turn.resetBeforeMessageId = turn.state.messages.at(-1)?.id;
              this.changed();
              // Fetch the reset timestamp so a prior failed turn keeps its note
              // before this turn's user message; a rebind moves it before the reply.
              void this.history(key).then((refreshed) => {
                if (generation !== this.generation || this.live.get(key) !== turn) return;
                if (refreshed?.conversation.context_reset_at) {
                  turn.conversation = refreshed.conversation;
                  const user = [...refreshed.messages]
                    .reverse()
                    .find((message) => message.role === "user" && !message.steering);
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
                this.changed();
              }).catch(() => undefined);
            }
            if (event.event === "turn.notice" && event.code === "retrying") {
              turn.retrying = true;
            } else if (event.event !== "turn.notice") {
              turn.retrying = false;
            }
            if (event.event === "turn.notice" && event.code === "image_input_unavailable") {
              void this.history(key).catch(() => undefined);
              this.changed();
              continue;
            }
            turn.state = applyDirectTurnEvent(turn.state, event);
            if (event.event === "block.started" || event.event === "block.delta" || event.event === "block.completed") {
              turn.replyCursor = event.cursor;
            }
            this.changed();
            if (event.event === "turn.completed") {
              terminal = true;
              if (event.status === "failed" && event.error) {
                onTurnFailed?.(key, event.turn_id, event.error.code);
              }
            }
          }
          if (terminal) break;
        }
      } finally {
        controller.signal.removeEventListener("abort", cancel);
        await reader.cancel().catch(() => undefined);
        reader.releaseLock();
      }
      if (!terminal) {
        throw new Error("The connection was interrupted. The assistant may still be working.");
      }
    } catch (error) {
      if (responded && adopted && generation === this.generation && !turn.settled) {
        await this.history(key).catch(() => undefined);
      }
      // A history refresh can prove completion before a stalled reader does.
      if (turn.settled) {
        terminal = true;
        if (turn.settled.status === "failed" && turn.settled.error_code) {
          onTurnFailed?.(key, turn.settled.turn_id, turn.settled.error_code);
        }
        return;
      }
      // After a 2xx the server has stored the message and bound its uploads:
      // every later failure (deadline, size, parse, mismatch, early end) is admitted.
      if (!responded) throw error;
      // A failed reader can throw a browser's opaque "network error" after
      // admission. Keep that failure distinct from a rejected send, including
      // the admission marker that prevents the composer restoring uploads.
      const failure = new Error(withAssistantTrace(
        "The live reply was interrupted. NyxBot may still be working; check the chat before sending again.",
        traceId,
      ), { cause: error });
      // DOMException.message may be read-only. Preserve cancellation/type
      // classification without mutating errors supplied by the browser.
      if (error instanceof Error) failure.name = error.name;
      throw markAdmitted(failure);
    } finally {
      this.requests.delete(controller);
      if (generation === this.generation) {
        if (adopted) await this.history(key).catch(() => undefined);
        this.current(generation);
        // Keep text already received if the connection fails between checkpoints.
        const history = this.histories.get(key);
        const activeTurn = history?.conversation.active_turn;
        const reply = turn.state.messages.at(-1);
        if (!terminal && activeTurn && reply?.role === "assistant"
          && activeTurn.turn_id === turn.state.activeTurn?.turnId
          && (turn.replyCursor ?? 0) > (activeTurn.reply?.cursor ?? 0)) {
          const block = reply.blocks[0];
          if (block) activeTurn.reply = { message_id: reply.id, block_id: block.block_id,
            text: reply.blocks.map((row) => row.text).join("\n\n"), cursor: turn.replyCursor! };
        }
        this.live.delete(key);
        if (!terminal && this.getHistory(key)?.conversation.active_turn) this.interrupted.add(key);
        else this.interrupted.delete(key);
        this.changed();
      }
    }
  }

  /** Retry the failed reply `messageId`. The server fences the calls that
   * failed turn completed and re-attaches its input's uploads (they are
   * already bound, so their ids are never re-sent). */
  private async refreshRecovery(id: string, messageId: string, generation: number) {
    const revision = this.rowRevisions.get(id);
    const source = this.histories.get(id)?.messages.find((message) => message.id === messageId);
    if (!source || source.role !== "assistant" || source.status !== "failed") {
      throw new Error("Failed reply not found. Refresh the conversation.");
    }
    let recovery;
    try {
      recovery = nyxAgentRecoverySchema.parse(
        await assistantJson(`${path(id)}/messages/${encodeURIComponent(messageId)}/recovery`),
      );
    } catch {
      const previous = this.histories.get(id)?.messages.find((message) => message.id === messageId)?.recovery;
      recovery = { status: "review" as const, reason: "status_unavailable", text: previous?.text ?? null,
        attachments: previous?.attachments ?? [] };
    }
    this.current(generation);
    if (this.consumedRetries.get(id)?.has(messageId)) {
      recovery = { ...recovery, status: "review" as const, reason: "already_retried" };
    }
    if (revision !== this.rowRevisions.get(id) ||
      this.histories.get(id)?.messages.find((message) => message.id === messageId) !== source) {
      throw new DOMException("Conversation changed", "AbortError");
    }
    const history = this.histories.get(id);
    if (history && history.messages.some((message) => message.id === messageId)) {
      this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
      this.histories.set(id, {
        ...history,
        messages: history.messages.map((message) => message.id === messageId
          ? { ...message, recovery } : message),
      });
      this.changed();
    }
    return recovery;
  }

  async checkRecovery(id: string, messageId: string) {
    return this.refreshRecovery(id, messageId, this.identity());
  }

  private markRecoveryConsumed(id: string, messageId: string, turnId: string) {
    const history = this.histories.get(id);
    if (!history?.messages.some((message) => message.id === messageId && message.turn_id === turnId)) return;
    const consumed = this.consumedRetries.get(id) ?? new Set<string>();
    consumed.add(messageId);
    this.consumedRetries.set(id, consumed);
    this.rowRevisions.set(id, (this.rowRevisions.get(id) ?? 0) + 1);
    this.histories.set(id, { ...history, messages: history.messages.map((message) =>
      message.id === messageId && message.turn_id === turnId
        ? { ...message, recovery: { ...message.recovery, status: "review", reason: "already_retried",
          text: message.recovery?.text ?? null, attachments: message.recovery?.attachments ?? [] } }
        : message),
    });
    this.changed();
  }

  async retry(id: string, messageId: string, onAdopt: (id: string) => void) {
    const messages = this.histories.get(id)?.messages ?? [];
    const failed = messages.find(
      (message) => message.id === messageId && message.role === "assistant" && message.status === "failed",
    );
    if (!failed) throw new Error("Failed reply not found. Refresh the conversation.");
    const generation = this.identity();
    const recovery = await this.refreshRecovery(id, messageId, generation);
    const current = this.histories.get(id)?.messages.find((message) => message.id === messageId);
    if (!current || current.status !== "failed" || current.turn_id !== failed.turn_id) {
      throw new DOMException("Conversation changed", "AbortError");
    }
    if (recovery.status !== "available" || recovery.text === null) {
      throw new Error("This reply needs review before another attempt.");
    }
    let admitted = false;
    try {
      const result = await this.send(id, recovery.text, onAdopt, undefined, {
        adoptExisting: true,
        retryOf: failed.turn_id,
      });
      admitted = true;
      return result;
    } catch (error) {
      admitted = isAdmittedSendError(error);
      throw error;
    } finally {
      if (generation === this.generation) {
        if (admitted) this.markRecoveryConsumed(id, messageId, failed.turn_id);
        await this.refreshRecovery(id, messageId, generation).catch(() => undefined);
      }
    }
  }
}

export const nyxAgentTransport = new NyxAgentTransport();

function sameFields(a: ChatMessage, b: ChatMessage): boolean {
  const keys = Object.keys(a) as (keyof ChatMessage)[];
  return keys.length === Object.keys(b).length && keys.every((key) => a[key] === b[key]);
}

function provisionalTitle(text: string): string {
  const line = text.split(/\r?\n/).find((part) => part.trim()) ?? "";
  const clean = line.replace(/\s+/g, " ").trim();
  if ([...clean].length <= 60) return clean || "New chat";
  const prefix = [...clean].slice(0, 60).join("");
  return prefix.includes(" ") ? prefix.slice(0, prefix.lastIndexOf(" ")) : prefix;
}
