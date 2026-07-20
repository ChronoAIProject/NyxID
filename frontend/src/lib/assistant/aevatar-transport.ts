import { apiClient } from "@/lib/api-client";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import { drainSseBuffer, flushSseBuffer } from "@/lib/assistant/sse";
import {
  applyTurnEvent,
  EMPTY_TURN_STATE,
  toTerminalBlock,
} from "@/lib/assistant/stream";
import type {
  ApprovalCardContentBlock,
  AssistantMessage,
  AssistantTransport,
  ConnectCardContentBlock,
  ContentBlock,
  Conversation,
  ConversationHistory,
  RunContentBlock,
  TurnEvent,
  TurnHandle,
  TurnReducerState,
} from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";

// NyxID's own assistant pass-through (PRD decision 4). NyxID resolves the
// admin-managed aevatar service and derives the aevatar scope from the
// verified session user, so no scope segment appears here: the browser
// cannot name a scope, and the surface does not depend on the caller having
// personally connected aevatar.
const ASSISTANT_PREFIX = "/assistant";

// A 401 from the assistant mount means the downstream (aevatar) rejected the
// forwarded identity — page content failing, not the NyxID session dying — so
// every request opts out of the api-client's global sign-out (#1190) and the
// failure surfaces as a normal query error (toast + error state) instead of a
// redirect to /login. Scoped to this transport; the rest of the app keeps
// strict 401 handling.
const assistantApi = {
  get<T>(endpoint: string): Promise<T> {
    return apiClient<T>(endpoint, { preserveSessionOn401: true });
  },
  post<T>(endpoint: string, body?: unknown): Promise<T> {
    return apiClient<T>(endpoint, {
      method: "POST",
      body,
      preserveSessionOn401: true,
    });
  },
  del<T>(endpoint: string): Promise<T> {
    return apiClient<T>(endpoint, {
      method: "DELETE",
      preserveSessionOn401: true,
    });
  },
} as const;

// The conversation list reads the Chat History index (contract-B): each row
// already carries a server title, timestamps, and message count, so the list
// needs no per-conversation history hydration fan-out.
//
// Sidebar list re-fetch throttle: `projectTransportState` re-projects the
// conversation list after every turn event, which must not become one
// network round-trip per streamed token.
const CONVERSATION_LIST_TTL_MS = 5_000;

const MAX_MESSAGE_CHARS = 32_768;

// Reference-client parity (nyxid-chat BFF `DEMO_STREAM_PROGRESS_TIMEOUT_MS`):
// a stream that produces only keepalives for this long is a hung run, not a
// slow one. Suspended while a human approval gate is open — external gates
// have no deadline the client can impose.
const STREAM_PROGRESS_TIMEOUT_MS = 120_000;

// Inline media larger than this (base64 chars ≈ 6 MB decoded) is summarized
// as text instead of being embedded as a data: URL artifact.
const MAX_MEDIA_DATA_CHARS = 8_000_000;

const MAX_TOOL_SUMMARY_CHARS = 160;

/** AG-UI frame vocabulary observed on `nyxid-chat/conversations/{id}:stream`. */
interface ToolCallPayload {
  readonly toolCallId?: string;
  readonly toolName?: string;
  readonly status?: string;
  readonly success?: boolean;
  readonly result?: unknown;
  readonly error?: unknown;
}

interface ToolApprovalPayload {
  readonly requestId?: string;
  readonly approvalRequestId?: string;
  readonly toolCallId?: string;
  readonly toolName?: string;
  readonly message?: string;
  readonly body?: string;
  readonly serviceSlug?: string;
  readonly service_slug?: string;
  readonly agentKeyPrefix?: string;
  readonly approvalMode?: string;
  readonly grantDurationSec?: number;
  readonly expiresAt?: string;
  readonly expires_at?: string;
  readonly commandId?: string;
  readonly stepId?: string;
}

interface AuthorizationPayload {
  readonly serviceId?: string;
  readonly serviceSlug?: string;
  readonly service_slug?: string;
  readonly serviceLabel?: string;
  readonly resource?: string;
  readonly resourceUri?: string;
  readonly message?: string;
}

interface UsagePayload {
  readonly available?: boolean;
  readonly promptTokens?: number;
  readonly completionTokens?: number;
  readonly totalTokens?: number;
  readonly model?: string | null;
}

interface MediaPayload {
  readonly mediaType?: string;
  readonly dataBase64?: string;
  readonly url?: string;
  readonly name?: string;
}

interface StepPayload {
  readonly runId?: string;
  readonly stepId?: string;
  readonly stepType?: string;
  readonly success?: boolean;
}

interface CustomEnvelope {
  readonly name?: string;
  readonly payload?: unknown;
}

interface AgUiFrame {
  readonly type?: string;
  readonly actorId?: string;
  readonly textMessageStart?: {
    readonly messageId?: string;
    readonly role?: string;
  };
  readonly textMessageContent?: { readonly delta?: string };
  readonly textMessageEnd?: { readonly messageId?: string };
  readonly toolCallStart?: ToolCallPayload;
  readonly toolCallEnd?: ToolCallPayload;
  readonly toolApprovalRequest?: ToolApprovalPayload;
  readonly authorizationRequired?: AuthorizationPayload;
  readonly usage?: UsagePayload;
  readonly mediaContent?: MediaPayload;
  readonly custom?: CustomEnvelope;
  readonly runError?: { readonly code?: string; readonly message?: string };
  readonly error?: { readonly code?: string; readonly message?: string };
  readonly message?: string;
}

/** Batched completion inside `aevatar.raw.observed` (reference `protocol.js`). */
interface RoleChatToolCall {
  readonly callId?: string;
  readonly toolName?: string;
  readonly argumentsJson?: string;
}

interface RoleChatToolReceipt {
  readonly callId?: string;
  readonly toolName?: string;
  readonly status?: string;
  readonly resultJson?: string;
  readonly errorMessage?: string;
  readonly errorCode?: string;
}

interface RoleChatCompletion {
  readonly sessionId?: string;
  readonly content?: string;
  readonly toolCalls?: readonly RoleChatToolCall[];
  readonly toolReceipts?: readonly RoleChatToolReceipt[];
  readonly usage?: UsagePayload;
  readonly model?: string;
}

/** One row of the Chat History index (`chat-history`). */
interface AevatarHistoryIndexEntry {
  readonly id?: string;
  readonly title?: string;
  readonly createdAt?: string;
  readonly updatedAt?: string;
  readonly messageCount?: number;
  readonly llmRoute?: string | null;
  readonly llmModel?: string | null;
}

interface AevatarConversationListResponse {
  readonly conversations?: readonly AevatarHistoryIndexEntry[];
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
  /**
   * Aevatar run-session correlation id, minted once per conversation and
   * sent on every `:stream` body (the reference client keeps one session
   * per conversation; the field is optional upstream).
   */
  sessionId?: string;
}

interface RunStepState {
  index: number;
  status: "done" | "active" | "waiting" | "failed" | "skipped";
  label: string;
  meta: string;
  service_slug: string | null;
  artifact_id: string | null;
  approval_request_id: string | null;
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
  /** Any assistant text streamed this turn (gates the batched-content fallback). */
  sawText: boolean;
  /** Synthetic assistant message hosting the run ledger and cards. */
  activityMessageId: string | null;
  activityBlockCount: number;
  runBlockId: string | null;
  runSteps: RunStepState[];
  /** tool `toolCallId` / workflow `stepId` → index into `runSteps`. */
  stepKeys: Map<string, number>;
  /** Open cards: block_id → kind, completed at turn finalization. */
  openCards: Map<string, "approval" | "connect">;
  /** Dedupe guards, per reference client behavior. */
  promptedApprovalIds: Set<string>;
  promptedConnectSlugs: Set<string>;
  waitingForApproval: boolean;
  watchdog: ReturnType<typeof setTimeout> | null;
}

function newId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

function noopEvent(): void {
  // Continuation stream with no subscriber: state still lands in the
  // transport mirror via `emit`.
}

function isoFromEpochMs(epochMs: number | undefined, fallback: string): string {
  if (typeof epochMs !== "number" || !Number.isFinite(epochMs)) {
    return fallback;
  }
  return new Date(epochMs).toISOString();
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
 * Map a pre-stream rejection to a turn error. Errors before the SSE stream
 * starts arrive as a JSON envelope — NyxID's `{error, error_code, message}`
 * or Aevatar's `{code, message}` — and must not collapse to a bare
 * `http_<status>`. A 401/403 here means the downstream rejected the
 * identity NyxID forwarded, not that the NyxID session died — this raw
 * fetch never touches auth state, so it cannot trigger a sign-out; the
 * copy says so explicitly.
 */
function streamStartError(
  status: number,
  bodyText: string,
): { code: string; message: string } {
  interface ErrorEnvelope {
    readonly error?: unknown;
    readonly code?: unknown;
    readonly message?: unknown;
  }
  let envelope: ErrorEnvelope | null = null;
  try {
    envelope = JSON.parse(bodyText) as ErrorEnvelope;
  } catch {
    envelope = null;
  }
  const envelopeCode =
    typeof envelope?.code === "string" && envelope.code
      ? envelope.code
      : typeof envelope?.error === "string" && envelope.error
        ? envelope.error
        : null;
  const envelopeMessage =
    typeof envelope?.message === "string" && envelope.message.trim()
      ? envelope.message
      : null;
  if (status === 401 || status === 403) {
    return {
      code: envelopeCode ?? "unauthorized",
      message:
        "NyxID could not authenticate you to the chat backend. You are still signed in — reconnect the aevatar service and try again.",
    };
  }
  return {
    code: envelopeCode ?? `http_${String(status)}`,
    message:
      envelopeMessage ?? "The assistant stream could not be started.",
  };
}

// ---------------------------------------------------------------------------
// Redaction and tool-result summaries (ported from the reference client's
// `protocol.js`): display strings derived from tool payloads are untrusted
// and may echo credentials; PRD §3.6 forbids raw downstream bodies in blocks.
// ---------------------------------------------------------------------------

const SECRET_VALUE_PATTERN =
  /(Bearer\s+)[A-Za-z0-9._~+/-]+|\beyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b|nyx(?:id)?_[A-Za-z0-9_-]{8,}/gi;

const SECRET_ASSIGNMENT_PATTERN =
  /("?(?:authorization|api[-_]?key|token|secret|password|credential|cookie)"?\s*[:=]\s*)"?[^",\s}]+"?/gi;

export function redactDisplayText(value: string): string {
  // Token shapes first: the assignment rule would otherwise consume the
  // literal "Bearer" as the assigned value and leave the token itself
  // (`Authorization: Bearer <token>`) untouched.
  return value
    .replace(SECRET_VALUE_PATTERN, (_match, bearerPrefix: unknown) =>
      typeof bearerPrefix === "string" && bearerPrefix
        ? `${bearerPrefix}[redacted]`
        : "[redacted]",
    )
    .replace(SECRET_ASSIGNMENT_PATTERN, '$1"[redacted]"');
}

/** Compact, redacted single-line summary of a tool result for the step meta. */
export function summarizeToolResult(value: unknown): string {
  if (value === undefined || value === null || value === "") {
    return "Completed";
  }
  let text: string;
  if (typeof value === "string") {
    text = value;
  } else {
    try {
      text = JSON.stringify(value);
    } catch {
      return "Completed";
    }
  }
  const compact = redactDisplayText(text).replace(/\s+/g, " ").trim();
  if (!compact) return "Completed";
  return compact.length > MAX_TOOL_SUMMARY_CHARS
    ? `${compact.slice(0, MAX_TOOL_SUMMARY_CHARS - 1)}…`
    : compact;
}

// ---------------------------------------------------------------------------
// Authorization-failure sniffing (reference `findServiceAuthorizationFailure`):
// belt-and-braces below the dedicated AUTHORIZATION_REQUIRED frame — some
// runs only surface the gap inside a tool-result error string.
// ---------------------------------------------------------------------------

const AUTHORIZATION_FAILURE_MARKERS = [
  "authorization_required",
  "service_not_authorized",
  "service_access_required",
  "service authorization required",
  "not authorized for this service",
  "not authorized for service",
  "does not have access to this service",
  "does not have access to service",
  "service access is not granted",
  "scoped api keys must use configured services",
  "invalid_target",
] as const;

export function findServiceAuthorizationFailure(
  value: unknown,
): { serviceSlug: string; message: string } | null {
  if (value === undefined || value === null || value === "") return null;
  let text: string;
  try {
    text = (typeof value === "string" ? value : JSON.stringify(value))
      .toLowerCase();
  } catch {
    return null;
  }
  if (!AUTHORIZATION_FAILURE_MARKERS.some((marker) => text.includes(marker))) {
    return null;
  }
  let explicitSlug = "";
  if (typeof value === "object" && value !== null) {
    const record = value as Record<string, unknown>;
    const candidate = record["serviceSlug"] ?? record["service_slug"];
    if (typeof candidate === "string") explicitSlug = candidate;
  }
  const slug =
    explicitSlug.trim().toLowerCase() ||
    (/(?:service|slug)[\s"':=]+([a-z0-9][a-z0-9-]{1,80})/.exec(text)?.[1] ??
      "");
  return {
    serviceSlug: slug,
    message: slug
      ? `This request needs a working ${slug} connection. Connect it in NyxID, then ask again.`
      : "This request needs a NyxID service that is not connected yet. Connect it, then ask again.",
  };
}

/**
 * Unwrap a protobuf-`Any`-shaped custom payload (reference `unpackAny`):
 * either `{value: {...}}` or the object itself minus its `@type` marker.
 */
function unpackAny(payload: unknown): Record<string, unknown> {
  if (typeof payload !== "object" || payload === null) return {};
  const record = payload as Record<string, unknown>;
  const value = record["value"];
  if (typeof value === "object" && value !== null) {
    return value as Record<string, unknown>;
  }
  const clone: Record<string, unknown> = { ...record };
  delete clone["@type"];
  return clone;
}

function humanizeSlug(slug: string): string {
  const bare = slug.replace(/^api-/, "");
  if (!bare) return "NyxID service";
  return bare
    .split(/[-_]/)
    .map((part) => (part ? part.charAt(0).toUpperCase() + part.slice(1) : part))
    .join(" ");
}

function base64SizeBytes(dataBase64: string): number {
  return Math.floor((dataBase64.length * 3) / 4);
}

/**
 * Real `AssistantTransport` backed by Aevatar's nyxid-chat API (PRD C1
 * provider). Conversation state is mirrored in a client-side store so
 * streaming turns render live from turn events while the server history
 * (`chat-history/conversations/{id}`) stays authoritative between turns —
 * the same authority split the mock transport established.
 *
 * The stream handler adapts the FULL live AG-UI vocabulary (the reference
 * client's `protocol.js` taxonomy) onto the PRD §3.5 block types the UI
 * renders: tool calls become the `run` step ledger, approval requests become
 * `approval_card`, authorization gaps become `connect_card`, inline media
 * becomes `artifact`. Reasoning frames are acknowledged but never rendered
 * (PRD §3.8); batched `aevatar.raw.observed` completions are mined for their
 * presentation content only.
 */
export class AevatarAssistantTransport implements AssistantTransport {
  private readonly conversations = new Map<string, StoredConversation>();
  private readonly running = new Map<string, RunningTurn>();
  private listFetchedAt = 0;

  async listConversations(): Promise<Conversation[]> {
    const now = Date.now();
    if (
      this.running.size === 0 &&
      now - this.listFetchedAt > CONVERSATION_LIST_TTL_MS
    ) {
      this.listFetchedAt = now;
      const response = await assistantApi.get<AevatarConversationListResponse>(
        `${ASSISTANT_PREFIX}/conversations`,
      );
      for (const entry of response.conversations ?? []) {
        const id = entry?.id?.trim();
        if (id) this.mergeIndexEntry(id, entry);
      }
    }
    return [...this.conversations.values()]
      .map((stored) => stored.conversation)
      .sort((a, b) => b.last_message_at.localeCompare(a.last_message_at));
  }

  async createConversation(): Promise<Conversation> {
    const response = await assistantApi.post<AevatarCreateConversationResponse>(
      `${ASSISTANT_PREFIX}/conversations`,
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

  async deleteConversation(conversationId: string): Promise<void> {
    // A turn still streaming into the conversation must not keep painting
    // a transcript that is about to disappear: cancel first, matching the
    // reference client's abort-then-delete order. The server side is the
    // #1199 composite delete (actor + history row, 404-tolerant), so one
    // call retires both upstream surfaces.
    const run = this.running.get(conversationId);
    if (run) this.cancelTurn(conversationId, run);
    await assistantApi.del(
      `${ASSISTANT_PREFIX}/conversations/${conversationId}`,
    );
    // Local removal only after the server accepted: a failed delete keeps
    // the conversation listed and retryable.
    this.conversations.delete(conversationId);
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

    const run = this.newRun(onEvent);
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

  /**
   * Send an approval decision. On the live contract the approve endpoint
   * answers with an SSE continuation of the run (reference client behavior);
   * the frames stream through the same adapter as the original turn, with
   * cursors continuing past the previous turn's so at-least-once consumers
   * never regress. Resolves once the continuation has started; the returned
   * handle cancels it.
   */
  async decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
    onEvent?: (event: TurnEvent) => void,
  ): Promise<TurnHandle | null> {
    const stored = this.conversations.get(conversationId);
    const card = stored?.turnState.messages
      .flatMap((message) => message.blocks)
      .find(
        (block): block is ApprovalCardContentBlock =>
          block.type === "approval_card" && block.block_id === blockId,
      );
    if (!stored || !card) {
      throw new Error("Approval request was not found.");
    }
    if (card.decision !== null) {
      throw new Error("This approval was already decided.");
    }
    const active = this.running.get(conversationId);
    if (active) {
      if (!active.waitingForApproval) {
        throw new AssistantTurnActiveError();
      }
      // The original stream is idle at the human gate; settle it quietly so
      // the continuation below becomes the one live turn.
      this.pauseForApproval(conversationId, active);
    }

    const response = await fetch(
      `/api/v1${ASSISTANT_PREFIX}/conversations/${conversationId}/approve`,
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "text/event-stream",
        },
        credentials: "include",
        body: JSON.stringify({
          requestId: card.approval_request_id,
          approved,
          ...(stored.sessionId ? { sessionId: stored.sessionId } : {}),
        }),
      },
    );
    if (!response.ok) {
      const bodyText = await response.text().catch(() => "");
      throw new Error(streamStartError(response.status, bodyText).message);
    }

    const run = this.newRun(onEvent ?? noopEvent);
    // Continuation cursors continue past the previous turn's: the reducer
    // and any still-subscribed pump dedup by strictly-increasing cursor.
    run.cursor = stored.turnState.lastCursor;
    this.running.set(conversationId, run);
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.status",
      turn_id: run.turnId,
      status: "running",
    });
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.updated",
      block_id: blockId,
      patch: {
        decision: approved ? "approved" : "denied",
        decision_channel: "web",
      },
    });

    const contentType = response.headers.get("content-type") ?? "";
    if (response.body && contentType.includes("text/event-stream")) {
      void this.consumeTurnStream(conversationId, run, response);
    } else {
      // Older backend acknowledging with JSON: nothing further will stream.
      this.finishTurn(conversationId, run, "completed", null);
    }
    return {
      turnId: run.turnId,
      cancel: () => {
        this.cancelTurn(conversationId, run);
      },
    };
  }

  /**
   * Fold one Chat History index row into the local mirror. A conversation
   * with a live turn keeps its in-flight mirror (index metadata must not
   * clobber a streaming transcript); everything else adopts the server's
   * title, timestamps, and counts without a per-conversation detail fetch.
   */
  private mergeIndexEntry(id: string, entry: AevatarHistoryIndexEntry): void {
    const existing = this.conversations.get(id);
    if (existing && isTurnActive(existing.turnState.activeTurn?.status)) {
      return;
    }
    const epoch0 = new Date(0).toISOString();
    const createdAt =
      entry.createdAt ?? existing?.conversation.created_at ?? epoch0;
    const lastMessageAt =
      entry.updatedAt ??
      entry.createdAt ??
      existing?.conversation.last_message_at ??
      createdAt;
    const title =
      entry.title?.trim() || existing?.conversation.title || "Conversation";
    const conversation: Conversation = {
      id,
      title,
      created_at: createdAt,
      last_message_at: lastMessageAt,
      message_count:
        typeof entry.messageCount === "number"
          ? entry.messageCount
          : existing?.conversation.message_count,
      llm_route: entry.llmRoute ?? existing?.conversation.llm_route ?? null,
      llm_model: entry.llmModel ?? existing?.conversation.llm_model ?? null,
    };
    this.conversations.set(id, {
      conversation,
      turnState: existing?.turnState ?? EMPTY_TURN_STATE,
      // Reprojection must not re-mint the run-session id: one session per
      // conversation is the reference contract.
      sessionId: existing?.sessionId,
    });
  }

  private async loadHistory(
    conversationId: string,
  ): Promise<StoredConversation> {
    const entries = await assistantApi.get<AevatarHistoryEntry[]>(
      `${ASSISTANT_PREFIX}/conversations/${conversationId}`,
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
      // Post-turn history reloads run through here; losing the session id
      // would silently start a new upstream run-session every turn.
      sessionId: existing?.sessionId,
    };
    this.conversations.set(conversationId, stored);
    return stored;
  }

  private newRun(onEvent: (event: TurnEvent) => void): RunningTurn {
    return {
      turnId: newId("turn"),
      controller: new AbortController(),
      onEvent,
      cursor: 0,
      currentMessageId: null,
      currentBlockId: null,
      accumulatedText: "",
      finished: false,
      sawText: false,
      activityMessageId: null,
      activityBlockCount: 0,
      runBlockId: null,
      runSteps: [],
      stepKeys: new Map(),
      openCards: new Map(),
      promptedApprovalIds: new Set(),
      promptedConnectSlugs: new Set(),
      waitingForApproval: false,
      watchdog: null,
    };
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
      this.clearWatchdog(run);
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
      const stored = this.conversations.get(conversationId);
      const sessionId = stored && (stored.sessionId ??= crypto.randomUUID());
      // Hand-rolled fetch (not `apiClient`): the response is an SSE stream,
      // and the endpoint 415s without an explicit JSON content type.
      const response = await fetch(
        `/api/v1${ASSISTANT_PREFIX}/conversations/${conversationId}/stream`,
        {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Accept: "text/event-stream",
          },
          credentials: "include",
          body: JSON.stringify(sessionId ? { prompt, sessionId } : { prompt }),
          signal: run.controller.signal,
        },
      );
      if (!response.ok || !response.body) {
        const bodyText = await response.text().catch(() => "");
        this.finishTurn(
          conversationId,
          run,
          "failed",
          streamStartError(response.status, bodyText),
        );
        return;
      }
      await this.consumeTurnStream(conversationId, run, response);
    } catch (error) {
      // The cancel path aborts the fetch after emitting its own terminal
      // events; an abort here is not a failure.
      if (run.finished || run.controller.signal.aborted) return;
      this.closeOpenMessage(conversationId, run);
      this.finalizeActivity(conversationId, run, "failed");
      this.finishTurn(conversationId, run, "failed", {
        code: "network_error",
        message:
          error instanceof Error ? error.message : "The stream failed.",
      });
    }
  }

  /**
   * Shared SSE consumption for the original turn stream and the approval
   * continuation stream: same framing, same adapter, same watchdog and
   * truncation semantics.
   */
  private async consumeTurnStream(
    conversationId: string,
    run: RunningTurn,
    response: Response,
  ): Promise<void> {
    if (!response.body) {
      this.finishTurn(conversationId, run, "completed", null);
      return;
    }
    try {
      this.armWatchdog(conversationId, run);
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
      // The final frame may arrive without a trailing blank line; flush it
      // before judging how the stream ended.
      buffer += decoder.decode();
      for (const payload of flushSseBuffer(buffer)) {
        this.handleAgUiFrame(conversationId, run, payload);
        if (run.finished) return;
      }
      this.closeOpenMessage(conversationId, run);
      if (run.waitingForApproval) {
        // EOF at a human gate is a pause, not a truncation: Aevatar may
        // close an idle stream while an approval waits (PRD §3.4). The card
        // stays actionable; the decision starts the continuation stream.
        this.finalizeActivity(conversationId, run, "waiting");
        this.finishTurn(conversationId, run, "completed", null);
        return;
      }
      // EOF without RUN_FINISHED / RUN_ERROR is a truncated run (proxy idle
      // kill, upstream drop), not a success. Settle the partial state and
      // report it; the server-side run may still finish, in which case the
      // full reply surfaces on the next history reload.
      this.finalizeActivity(conversationId, run, "failed");
      this.finishTurn(conversationId, run, "failed", {
        code: "stream_closed",
        message:
          "The stream ended before the assistant finished. The reply may be incomplete — it will appear in full once the conversation reloads.",
      });
    } catch (error) {
      if (run.finished || run.controller.signal.aborted) return;
      this.closeOpenMessage(conversationId, run);
      this.finalizeActivity(conversationId, run, "failed");
      this.finishTurn(conversationId, run, "failed", {
        code: "network_error",
        message:
          error instanceof Error ? error.message : "The stream failed.",
      });
    } finally {
      this.clearWatchdog(run);
    }
  }

  // -------------------------------------------------------------------------
  // Watchdog (G-hang): keepalives keep the connection alive but are not
  // progress; only real frames re-arm the timer.
  // -------------------------------------------------------------------------

  private armWatchdog(conversationId: string, run: RunningTurn): void {
    this.clearWatchdog(run);
    if (run.finished || run.waitingForApproval) return;
    run.watchdog = setTimeout(() => {
      this.closeOpenMessage(conversationId, run);
      this.finalizeActivity(conversationId, run, "failed");
      this.finishTurn(conversationId, run, "failed", {
        code: "upstream_progress_timeout",
        message: `The assistant made no progress for ${String(
          Math.round(STREAM_PROGRESS_TIMEOUT_MS / 1000),
        )} seconds and the run was stopped. Try again.`,
      });
      run.controller.abort();
    }, STREAM_PROGRESS_TIMEOUT_MS);
  }

  private clearWatchdog(run: RunningTurn): void {
    if (run.watchdog !== null) {
      clearTimeout(run.watchdog);
      run.watchdog = null;
    }
  }

  // -------------------------------------------------------------------------
  // Frame adapter: live AG-UI vocabulary → PRD §3.5 blocks / §3.7 events.
  // -------------------------------------------------------------------------

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
    if (typeof frame !== "object" || frame === null) return;

    const type = this.frameType(frame);
    const isKeepalive =
      type === "CUSTOM" &&
      frame.custom?.name === "aevatar.nyxid_chat.keepalive";
    if (!isKeepalive) {
      this.armWatchdog(conversationId, run);
    }

    switch (type) {
      case "RUN_STARTED":
        // Context only (actorId echoes the conversation); nothing renders.
        return;
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
        run.sawText = true;
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
      case "TOOL_CALL_START": {
        const start = frame.toolCallStart ?? {};
        this.startRunStep(
          conversationId,
          run,
          start.toolCallId ?? newId("tool"),
          start.toolName ?? "tool",
        );
        return;
      }
      case "TOOL_CALL_END": {
        this.finishToolCall(conversationId, run, frame.toolCallEnd ?? {});
        return;
      }
      case "TOOL_APPROVAL_REQUEST": {
        this.addApprovalCard(
          conversationId,
          run,
          frame.toolApprovalRequest ?? {},
        );
        return;
      }
      case "AUTHORIZATION_REQUIRED": {
        this.addConnectCard(
          conversationId,
          run,
          frame.authorizationRequired ?? {},
        );
        return;
      }
      case "USAGE": {
        this.applyUsage(conversationId, frame.usage ?? {});
        return;
      }
      case "MEDIA_CONTENT": {
        this.addMediaArtifact(conversationId, run, frame.mediaContent ?? {});
        return;
      }
      case "RUN_ERROR": {
        const error = frame.runError ?? frame.error;
        this.closeOpenMessage(conversationId, run);
        this.finalizeActivity(conversationId, run, "failed");
        this.finishTurn(conversationId, run, "failed", {
          code: error?.code ?? "run_error",
          message:
            error?.message ?? frame.message ?? "The assistant run failed.",
        });
        return;
      }
      case "RUN_FINISHED": {
        this.closeOpenMessage(conversationId, run);
        this.finalizeActivity(
          conversationId,
          run,
          run.waitingForApproval ? "waiting" : "done",
        );
        this.finishTurn(conversationId, run, "completed", null);
        return;
      }
      case "CUSTOM": {
        this.handleCustomFrame(conversationId, run, frame.custom ?? {});
        return;
      }
      default:
        // Any newer AG-UI frame type has no presentation mapping; skipping
        // is the §3.0 forward-compat posture (never drop the turn over an
        // unknown frame).
        return;
    }
  }

  /** Frame type, tolerating both `type`-tagged and body-keyed variants. */
  private frameType(frame: AgUiFrame): string {
    if (frame.type) return frame.type.toUpperCase();
    if (frame.textMessageStart) return "TEXT_MESSAGE_START";
    if (frame.textMessageContent) return "TEXT_MESSAGE_CONTENT";
    if (frame.textMessageEnd) return "TEXT_MESSAGE_END";
    if (frame.toolCallStart) return "TOOL_CALL_START";
    if (frame.toolCallEnd) return "TOOL_CALL_END";
    if (frame.toolApprovalRequest) return "TOOL_APPROVAL_REQUEST";
    if (frame.authorizationRequired) return "AUTHORIZATION_REQUIRED";
    if (frame.usage) return "USAGE";
    if (frame.mediaContent) return "MEDIA_CONTENT";
    if (frame.runError) return "RUN_ERROR";
    if (frame.custom) return "CUSTOM";
    return "UNKNOWN";
  }

  private handleCustomFrame(
    conversationId: string,
    run: RunningTurn,
    custom: CustomEnvelope,
  ): void {
    const name = custom.name ?? "";
    const payload = unpackAny(custom.payload);
    switch (name) {
      case "aevatar.nyxid_chat.keepalive":
        // Connection liveness only — deliberately not progress (watchdog).
        return;
      case "aevatar.llm.reasoning":
        // Reasoning exists on the wire but is never rendered or mirrored
        // (reference client records it as "[not displayed]"; PRD §3.8).
        return;
      case "aevatar.authorization.required":
      case "nyxid.authorization.required":
        this.addConnectCard(
          conversationId,
          run,
          payload as AuthorizationPayload,
        );
        return;
      case "aevatar.tool_approval.pending": {
        const approval = payload as ToolApprovalPayload;
        this.addApprovalCard(conversationId, run, approval);
        this.markStepWaiting(
          conversationId,
          run,
          approval.toolCallId ?? approval.stepId ?? "",
          approval.approvalRequestId ?? approval.requestId ?? "",
        );
        return;
      }
      case "aevatar.human_input.request":
        this.addApprovalCard(
          conversationId,
          run,
          payload as ToolApprovalPayload,
        );
        return;
      case "aevatar.step.request": {
        const step = payload as StepPayload;
        const key = step.stepId ?? step.stepType ?? newId("step");
        this.startRunStep(conversationId, run, key, key);
        return;
      }
      case "aevatar.step.completed": {
        const step = payload as StepPayload;
        this.finishRunStep(
          conversationId,
          run,
          step.stepId ?? "",
          step.success !== false,
          step.success === false ? "Step failed" : "Completed",
        );
        return;
      }
      case "aevatar.workflow.waiting_signal":
        this.emit(conversationId, run, {
          cursor: this.nextCursor(run),
          event: "turn.status",
          turn_id: run.turnId,
          status: "waiting",
        });
        return;
      case "aevatar.run.context":
      case "demo.conversation.context":
        // Correlation ids only; nothing renders.
        return;
      case "aevatar.raw.observed":
        this.applyObservedCompletion(conversationId, run, payload);
        return;
      default:
        return;
    }
  }

  /**
   * Batched run completion (`RoleChatSessionCompletedEvent`): the engine
   * envelope itself is telemetry the browser must not render (PRD §3.8), so
   * only its presentation content is mined — tool calls/receipts into the
   * step ledger, the final content as fallback text when nothing streamed,
   * and the model name. `reasoningContent` is never read.
   */
  private applyObservedCompletion(
    conversationId: string,
    run: RunningTurn,
    payload: Record<string, unknown>,
  ): void {
    const typeUrl =
      typeof payload["payloadTypeUrl"] === "string"
        ? payload["payloadTypeUrl"]
        : "";
    const observedType = typeUrl.split(/[/.]/).at(-1) ?? "";
    if (observedType !== "RoleChatSessionCompletedEvent") return;
    const completion = unpackAny(payload["payload"]) as RoleChatCompletion;

    const receipts = new Map(
      (completion.toolReceipts ?? [])
        .filter((receipt) => receipt.callId)
        .map((receipt) => [receipt.callId ?? "", receipt]),
    );
    for (const call of completion.toolCalls ?? []) {
      const callId = call.callId ?? newId("tool");
      this.startRunStep(conversationId, run, callId, call.toolName ?? "tool");
      const receipt = receipts.get(callId);
      receipts.delete(callId);
      this.applyToolReceipt(conversationId, run, callId, receipt);
    }
    // Receipts without a matching call (defensive: the reference client
    // renders both sides independently).
    for (const [callId, receipt] of receipts) {
      this.startRunStep(
        conversationId,
        run,
        callId,
        receipt.toolName ?? "tool",
      );
      this.applyToolReceipt(conversationId, run, callId, receipt);
    }

    if (completion.content && !run.sawText) {
      this.emitStaticText(conversationId, run, completion.content);
    }
    this.applyUsage(conversationId, {
      ...(completion.usage ?? {}),
      model: completion.model ?? completion.usage?.model ?? null,
    });
  }

  private applyToolReceipt(
    conversationId: string,
    run: RunningTurn,
    callId: string,
    receipt: RoleChatToolReceipt | undefined,
  ): void {
    if (!receipt) {
      this.finishRunStep(conversationId, run, callId, true, "Completed");
      return;
    }
    const failed = /(ERROR|DENIED)/i.test(receipt.status ?? "");
    this.finishRunStep(
      conversationId,
      run,
      callId,
      !failed,
      summarizeToolResult(
        failed
          ? (receipt.errorMessage ?? receipt.errorCode ?? "Failed")
          : receipt.resultJson,
      ),
    );
    this.sniffAuthorizationFailure(
      conversationId,
      run,
      receipt.resultJson ?? receipt.errorMessage,
    );
  }

  // -------------------------------------------------------------------------
  // Activity message: one synthetic assistant message per turn hosts the run
  // ledger and any cards/artifacts (the reference client's activity card),
  // so tool progress renders even on turns with no streamed text.
  // -------------------------------------------------------------------------

  private ensureActivityMessage(
    conversationId: string,
    run: RunningTurn,
  ): string {
    if (run.activityMessageId) return run.activityMessageId;
    const messageId = newId("assistant-activity");
    run.activityMessageId = messageId;
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "message.started",
      message_id: messageId,
      role: "assistant",
    });
    return messageId;
  }

  private appendActivityBlock(
    conversationId: string,
    run: RunningTurn,
    block: ContentBlock,
  ): void {
    const messageId = this.ensureActivityMessage(conversationId, run);
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.started",
      message_id: messageId,
      block_id: block.block_id,
      index: run.activityBlockCount,
      block,
    });
    run.activityBlockCount += 1;
  }

  private runBlockSnapshot(run: RunningTurn): RunContentBlock {
    const stepsComplete = run.runSteps.filter(
      (step) => step.status === "done",
    ).length;
    const state = run.waitingForApproval
      ? "awaiting_approval"
      : run.promptedConnectSlugs.size > 0
        ? "awaiting_connection"
        : "running";
    return {
      type: "run",
      block_id: run.runBlockId ?? newId("run-block"),
      title: "RUN",
      steps_total: run.runSteps.length,
      steps_complete: stepsComplete,
      state,
      steps: run.runSteps.map((step) => ({ ...step })),
    };
  }

  private patchRunBlock(conversationId: string, run: RunningTurn): void {
    if (!run.runBlockId) {
      const block = this.runBlockSnapshot(run);
      run.runBlockId = block.block_id;
      this.appendActivityBlock(conversationId, run, block);
      return;
    }
    const snapshot = this.runBlockSnapshot(run);
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.updated",
      block_id: run.runBlockId,
      // §3.7 patch rule: a patch touching `steps` carries the whole array.
      patch: {
        steps_total: snapshot.steps_total,
        steps_complete: snapshot.steps_complete,
        state: snapshot.state,
        steps: snapshot.steps,
      },
    });
  }

  private startRunStep(
    conversationId: string,
    run: RunningTurn,
    key: string,
    label: string,
  ): void {
    if (run.stepKeys.has(key)) return;
    run.stepKeys.set(key, run.runSteps.length);
    run.runSteps.push({
      index: run.runSteps.length + 1,
      status: "active",
      label: redactDisplayText(label),
      meta: "Running",
      service_slug: null,
      artifact_id: null,
      approval_request_id: null,
    });
    this.patchRunBlock(conversationId, run);
  }

  private finishRunStep(
    conversationId: string,
    run: RunningTurn,
    key: string,
    succeeded: boolean,
    meta: string,
  ): void {
    let index = run.stepKeys.get(key);
    if (index === undefined) {
      // End without a start: create the row retroactively (reference client
      // behavior), then finish it.
      this.startRunStep(conversationId, run, key, key || "tool");
      index = run.stepKeys.get(key);
      if (index === undefined) return;
    }
    const step = run.runSteps[index];
    if (!step) return;
    step.status = succeeded ? "done" : "failed";
    step.meta = meta;
    this.patchRunBlock(conversationId, run);
  }

  private markStepWaiting(
    conversationId: string,
    run: RunningTurn,
    key: string,
    approvalRequestId: string,
  ): void {
    if (!key) return;
    const index = run.stepKeys.get(key);
    if (index === undefined) return;
    const step = run.runSteps[index];
    if (!step || step.status !== "active") return;
    step.status = "waiting";
    step.approval_request_id = approvalRequestId || null;
    this.patchRunBlock(conversationId, run);
  }

  private finishToolCall(
    conversationId: string,
    run: RunningTurn,
    payload: ToolCallPayload,
  ): void {
    const key = payload.toolCallId ?? "";
    const status = (payload.status ?? "").toUpperCase();
    const succeeded =
      payload.success !== false && !/(ERROR|DENIED)/.test(status);
    const outcome = payload.result ?? payload.error;
    if (!key && run.stepKeys.size === 0) {
      this.startRunStep(
        conversationId,
        run,
        newId("tool"),
        payload.toolName ?? "tool",
      );
    }
    this.finishRunStep(
      conversationId,
      run,
      key || [...run.stepKeys.keys()].at(-1) || "",
      succeeded,
      succeeded
        ? summarizeToolResult(payload.result)
        : summarizeToolResult(outcome),
    );
    this.sniffAuthorizationFailure(conversationId, run, outcome);
  }

  private sniffAuthorizationFailure(
    conversationId: string,
    run: RunningTurn,
    outcome: unknown,
  ): void {
    const failure = findServiceAuthorizationFailure(outcome);
    if (!failure) return;
    this.addConnectCard(conversationId, run, {
      serviceSlug: failure.serviceSlug,
      message: failure.message,
    });
  }

  // -------------------------------------------------------------------------
  // Cards and artifacts.
  // -------------------------------------------------------------------------

  private addApprovalCard(
    conversationId: string,
    run: RunningTurn,
    payload: ToolApprovalPayload,
  ): void {
    const requestId =
      payload.requestId ?? payload.approvalRequestId ?? payload.commandId ?? "";
    if (!requestId || run.promptedApprovalIds.has(requestId)) return;
    run.promptedApprovalIds.add(requestId);
    run.waitingForApproval = true;
    // A human gate has no client-imposed deadline; stop the watchdog.
    this.clearWatchdog(run);

    const body =
      payload.message ??
      payload.body ??
      (payload.toolName
        ? `The assistant wants to run ${redactDisplayText(payload.toolName)}.`
        : "The assistant is requesting your approval to continue.");
    const block: ApprovalCardContentBlock = {
      type: "approval_card",
      block_id: newId("approval-card"),
      approval_request_id: requestId,
      body: redactDisplayText(body),
      service_slug: payload.serviceSlug ?? payload.service_slug ?? "",
      agent_key_prefix: payload.agentKeyPrefix ?? "aevatar",
      approval_mode: payload.approvalMode === "grant" ? "grant" : "per_request",
      grant_duration_sec:
        typeof payload.grantDurationSec === "number"
          ? payload.grantDurationSec
          : null,
      // Empty when upstream sends none: the card omits the countdown for an
      // unparseable expiry rather than inventing a deadline.
      expires_at: payload.expiresAt ?? payload.expires_at ?? "",
      decision: null,
      decision_channel: null,
    };
    this.appendActivityBlock(conversationId, run, block);
    run.openCards.set(block.block_id, "approval");
    if (run.runBlockId) this.patchRunBlock(conversationId, run);
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.status",
      turn_id: run.turnId,
      status: "waiting",
    });
  }

  private addConnectCard(
    conversationId: string,
    run: RunningTurn,
    payload: AuthorizationPayload & { readonly message?: string },
  ): void {
    const rawSlug = (payload.serviceSlug ?? payload.service_slug ?? "").trim();
    const dedupeKey = rawSlug || "unknown-service";
    if (run.promptedConnectSlugs.has(dedupeKey)) return;
    run.promptedConnectSlugs.add(dedupeKey);

    const catalogSlug = rawSlug.replace(/^api-/, "");
    const serviceName = payload.serviceLabel?.trim() || humanizeSlug(rawSlug);
    const message =
      payload.message?.trim() ||
      `Connect ${serviceName} in NyxID, then send your request again.`;
    const block: ConnectCardContentBlock = {
      type: "connect_card",
      block_id: newId("connect-card"),
      catalog_slug: catalogSlug || "custom",
      service_name: serviceName,
      icon_url: "",
      subtitle: "Required by this request",
      // Display default only: the FE re-resolves actionable parameters from
      // the NyxID catalog (§4.3); the live frame does not carry auth_kind.
      auth_kind: "api_key",
      requested_scopes: [],
      key_id: null,
      granted_scopes: null,
      device_user_code: null,
      device_verification_url: null,
      state: "needs_connection",
      error_message: null,
      steps: [
        {
          title: `Connect ${serviceName}`,
          body: redactDisplayText(message),
          done: false,
        },
      ],
      footer: "Brokered by NyxID · configure in AI Services, then ask again",
    };
    this.appendActivityBlock(conversationId, run, block);
    run.openCards.set(block.block_id, "connect");
    if (run.runBlockId) this.patchRunBlock(conversationId, run);
  }

  private addMediaArtifact(
    conversationId: string,
    run: RunningTurn,
    payload: MediaPayload,
  ): void {
    const name = payload.name?.trim() || "attachment";
    const mime = payload.mediaType ?? "application/octet-stream";
    if (
      payload.dataBase64 &&
      payload.dataBase64.length <= MAX_MEDIA_DATA_CHARS
    ) {
      this.appendActivityBlock(conversationId, run, {
        type: "artifact",
        block_id: newId("artifact"),
        artifact_id: newId("media"),
        name,
        mime,
        size_bytes: base64SizeBytes(payload.dataBase64),
        preview: null,
        download_url: `data:${mime};base64,${payload.dataBase64}`,
      });
      return;
    }
    if (payload.url) {
      this.appendActivityBlock(conversationId, run, {
        type: "artifact",
        block_id: newId("artifact"),
        artifact_id: newId("media"),
        name,
        mime,
        size_bytes: 0,
        preview: null,
        download_url: payload.url,
      });
      return;
    }
    // Oversized or shapeless media: acknowledge without embedding.
    this.emitStaticText(
      conversationId,
      run,
      `The assistant produced an attachment (${name}) that is too large to display here.`,
    );
  }

  private applyUsage(conversationId: string, usage: UsagePayload): void {
    const model = usage.model;
    if (typeof model !== "string" || !model) return;
    const stored = this.conversations.get(conversationId);
    if (!stored) return;
    stored.conversation = { ...stored.conversation, llm_model: model };
  }

  /** Emit a complete text message in one started→completed sweep. */
  private emitStaticText(
    conversationId: string,
    run: RunningTurn,
    text: string,
  ): void {
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
      block: { type: "text", block_id: blockId, text },
    });
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.completed",
      block_id: blockId,
      block: { type: "text", block_id: blockId, text },
    });
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "message.completed",
      message_id: messageId,
    });
    run.sawText = true;
  }

  // -------------------------------------------------------------------------
  // Turn finalization.
  // -------------------------------------------------------------------------

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

  /**
   * Settle the activity message when the turn ends. Outcomes:
   * - `done`      — steps still active are marked done; ledger completes.
   * - `waiting`   — a human gate is open; the ledger parks in
   *                 `awaiting_approval` and undecided cards stay actionable.
   * - `failed`    — active steps fail; undecided approvals cancel; connect
   *                 cards stay actionable (they ARE the recovery path).
   * - `cancelled` — PRD §5.6: every open block reaches a terminal state.
   */
  private finalizeActivity(
    conversationId: string,
    run: RunningTurn,
    outcome: "done" | "waiting" | "failed" | "cancelled",
  ): void {
    if (run.runBlockId) {
      for (const step of run.runSteps) {
        if (step.status !== "active") continue;
        step.status =
          outcome === "done"
            ? "done"
            : outcome === "failed"
              ? "failed"
              : outcome === "cancelled"
                ? "skipped"
                : step.status;
        if (step.status === "done" && step.meta === "Running") {
          step.meta = "Completed";
        }
      }
      const snapshot = this.runBlockSnapshot(run);
      const state =
        outcome === "done"
          ? run.runSteps.some((step) => step.status === "failed")
            ? "failed"
            : "completed"
          : outcome === "waiting"
            ? "awaiting_approval"
            : outcome === "failed"
              ? "failed"
              : "cancelled";
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.completed",
        block_id: run.runBlockId,
        block: { ...snapshot, block_id: run.runBlockId, state },
      });
    }

    const stored = this.conversations.get(conversationId);
    for (const [blockId, kind] of run.openCards) {
      const block = stored?.turnState.messages
        .flatMap((message) => message.blocks)
        .find((candidate) => candidate.block_id === blockId);
      if (!block) continue;
      const terminal =
        outcome === "cancelled" ||
        (outcome === "failed" && kind === "approval")
          ? toTerminalBlock(block)
          : block;
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.completed",
        block_id: blockId,
        block: terminal,
      });
    }
    run.openCards.clear();

    if (run.activityMessageId) {
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "message.completed",
        message_id: run.activityMessageId,
      });
    }
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
   * Quietly settle a turn idling at a human gate so an approval decision can
   * become the live turn: no card is terminal-ized (the decision flow patches
   * it), the ledger parks as awaiting-approval, and the fetch aborts.
   */
  private pauseForApproval(conversationId: string, run: RunningTurn): void {
    if (run.finished) return;
    this.closeOpenMessage(conversationId, run);
    this.finalizeActivity(conversationId, run, "waiting");
    this.finishTurn(conversationId, run, "completed", null);
    run.controller.abort();
  }

  /**
   * Client-side stop: aevatar's nyxid-chat surface has no cancel endpoint,
   * so cancelling aborts the SSE fetch and settles the local turn per the
   * PRD stop-flow — every open block reaches a terminal state (§5.6). The
   * server-side run may still finish; its full reply then surfaces on the
   * next history reload.
   */
  private cancelTurn(conversationId: string, run: RunningTurn): void {
    if (run.finished) return;
    run.controller.abort();
    this.clearWatchdog(run);
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
    this.finalizeActivity(conversationId, run, "cancelled");
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.status",
      turn_id: run.turnId,
      status: "cancelled",
    });
    this.finishTurn(conversationId, run, "cancelled", null);
  }
}
