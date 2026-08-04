import { ApiError, apiClient } from "@/lib/api-client";
import {
  ACTION_REQUEST_CONFLICT_NOTE,
  composeBlockedUnsupportedNote,
  composeUnreportedCompletedNote,
} from "@/lib/assistant/action-notes";
import {
  AssistantConversationNotFoundError,
  AssistantProtocolError,
  AssistantTurnActiveError,
  AssistantTurnCancelledError,
} from "@/lib/assistant/errors";
import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import {
  CREATE_RECOVERY_BACKOFF_POLICY,
  PROJECTION_BACKOFF_POLICY,
  nextBackoffDelay,
} from "@/lib/assistant/backoff";
import {
  chatStreamClient,
  type ChatStreamCompletionResult,
  type ChatStreamRequestHandle,
} from "@/lib/assistant/chat-stream-worker-client";
import {
  CHAT_STREAM_MAX_BODY_BYTES,
  type ChatStreamFrame,
  type ChatStreamWireEvent,
} from "@/lib/assistant/chat-stream-worker-protocol";
import { WireBodyCapture } from "@/lib/assistant/wire-body-capture";
import {
  actionReportSchema,
  assistantActionRequestSchema,
  buildActionContinueBody,
  buildActionWakeBody,
  recoverUnsupportedAssistantActionRequest,
  type ActionContinueBody,
  type ActionReport,
  type ActionWakeBody,
  type AssistantActionRequest,
} from "@/schemas/assistant-actions";
import {
  applyTurnEvent,
  EMPTY_TURN_STATE,
  toTerminalBlock,
} from "@/lib/assistant/stream";
import type {
  ActionCardContentBlock,
  ActionCardStatus,
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
  TurnStatus,
  ProjectionReconcileOutcome,
} from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";
import {
  captureAssistantWireLogHeader,
  useAssistantWireLogStore,
} from "@/stores/assistant-wire-log-store";
import { useAuthStore } from "@/stores/auth-store";
import {
  adoptReceiptIdentity,
  advanceReceiptFence,
  deleteReceipt,
  findReceiptByConversation,
  findReceiptByPlaceholder,
  listDeletionIntents,
  recordCreateReceipt,
  recordDeletionIntent,
  retireReceiptAfterMaterialization,
  resolveDeletionIntent,
} from "@/stores/assistant-receipt-store";

// NyxID's own assistant pass-through (PRD decision 4). NyxID resolves the
// admin-managed aevatar service and derives the aevatar scope from the
// verified session user, so no scope segment appears here: the browser
// cannot name a scope, and the surface does not depend on the caller having
// personally connected aevatar.
const ASSISTANT_PREFIX = "/assistant";
const DEBUG_UPSTREAM_REQUEST_HEADER = "X-NyxID-Debug-Upstream";
const DEBUG_UPSTREAM_RESPONSE_HEADER = "X-NyxID-Debug-Upstream-Log";

function assistantWireLogOptions(): {
  readonly headers?: Record<string, string>;
  readonly onResponse?: (response: Response) => void;
} {
  const { featureEnabled, captureEnabled } =
    useAssistantWireLogStore.getState();
  if (!featureEnabled || !captureEnabled) return {};
  return {
    headers: { [DEBUG_UPSTREAM_REQUEST_HEADER]: "1" },
    onResponse: (response) => {
      try {
        const exchangeId = captureAssistantWireLogHeader(
          response.headers.get(DEBUG_UPSTREAM_RESPONSE_HEADER),
          "header",
          response.status,
        );
        if (!exchangeId) return;
        const clone = response.clone();
        void captureDeliveredResponseBody(exchangeId, clone).catch(() => {
          useAssistantWireLogStore
            .getState()
            .finalizeCapture(exchangeId, "network_error");
        });
      } catch {
        // Wire capture is diagnostic-only and cannot affect the API request.
      }
    },
  };
}

async function captureDeliveredResponseBody(
  exchangeId: string,
  response: Response,
): Promise<void> {
  const store = useAssistantWireLogStore.getState();
  if (!response.body) {
    store.attachResponseBody(exchangeId, "", 0, false);
    store.finalizeCapture(exchangeId, "complete");
    return;
  }

  const reader = response.body.getReader();
  const capture = new WireBodyCapture(CHAT_STREAM_MAX_BODY_BYTES);
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      capture.push(value);
      if (capture.truncated) {
        await reader.cancel().catch(() => undefined);
        break;
      }
    }
    const result = capture.finish();
    const latestStore = useAssistantWireLogStore.getState();
    latestStore.attachResponseBody(
      exchangeId,
      result.text,
      result.bytes,
      result.truncated,
    );
    latestStore.finalizeCapture(exchangeId, "complete");
  } catch {
    useAssistantWireLogStore
      .getState()
      .finalizeCapture(exchangeId, "network_error");
  } finally {
    reader.releaseLock();
  }
}

function attachStreamWireEvent(
  exchangeId: string,
  event: ChatStreamWireEvent,
): void {
  const store = useAssistantWireLogStore.getState();
  switch (event.type) {
    case "lines":
      store.attachWireLines(
        exchangeId,
        event.lines,
        event.bytes,
        event.truncated,
      );
      return;
    case "body":
      store.attachResponseBody(
        exchangeId,
        event.text,
        event.bytes,
        event.truncated,
      );
      return;
    case "end":
      store.finalizeCapture(exchangeId, event.outcome);
  }
}

// A 401 from the assistant mount means the downstream (aevatar) rejected the
// forwarded identity — page content failing, not the NyxID session dying — so
// every request opts out of the api-client's global sign-out (#1190) and the
// failure surfaces as a normal query error (toast + error state) instead of a
// redirect to /login. Scoped to this transport; the rest of the app keeps
// strict 401 handling.
const assistantApi = {
  get<T>(endpoint: string, signal?: AbortSignal): Promise<T> {
    const wireLog = assistantWireLogOptions();
    return apiClient<T>(endpoint, {
      headers: {
        Accept: "application/json",
        ...wireLog.headers,
      },
      preserveSessionOn401: true,
      signal,
      onResponse: wireLog.onResponse,
    });
  },
  post<T>(endpoint: string, body?: unknown): Promise<T> {
    return apiClient<T>(endpoint, {
      method: "POST",
      body,
      preserveSessionOn401: true,
      ...assistantWireLogOptions(),
    });
  },
  del<T>(endpoint: string): Promise<T> {
    return apiClient<T>(endpoint, {
      method: "DELETE",
      preserveSessionOn401: true,
      ...assistantWireLogOptions(),
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

// Chat History materializes after the stream terminal. Equal-length structured
// mirrors keep their exact identities during this grace. Longer local mirrors
// converge only on wrapped, fence-current responses containing the latest
// assistant turn; legacy arrays retain the conservative keep-max behavior.
const HISTORY_MATERIALIZATION_GRACE_MS = 15_000;

// New chats run on Aevatar's Workflow Studio chat engine: the turn body
// carries `workflow: "studio"`, which is the only thing that selects that
// engine upstream. Aevatar's `/api/chat` dispatches on the PRESENCE of a
// `type` discriminator (MainnetChatEndpoints.ClassifyRequestAsync) — a body
// with `type` goes to the typed NyxIdChat actor handler and never reaches
// the workflow engine, so the two surfaces are mutually exclusive per turn.
// Conversations that already have a `nyxid-chat-…` actor keep the typed
// surface so their transcripts stay continuable. Routing is by the
// server-owned conversation-id family.
const TYPED_CHAT_URL = "/api/v1/assistant/chat";
const WORKFLOW_CHAT_URL = "/api/v1/assistant/workflow-chat";

// Client-local placeholder minted before the typed surface returned its
// authoritative actor. New chats no longer mint one; it survives only so a
// stale `?c=nyxid-pending-…` URL from a pre-studio session still resolves to
// a clean not-found instead of a network error.
const LEGACY_PENDING_TYPED_CONVERSATION_PREFIX = "nyxid-pending-";

// Server conversation ids minted by the workflow chat's history reservation
// (`chatc-{hash[..32]}`).
const WORKFLOW_CONVERSATION_PREFIX = "chatc-";

// Client-local id for a workflow conversation that has not reached the
// server yet. The server id arrives in the first turn's
// `aevatar.chat.context` frame, which aliases this id to it; a reload
// forgets the placeholder and lists the server row instead.
const PENDING_WORKFLOW_CONVERSATION_PREFIX = "workflow-pending-";

function isWorkflowConversationId(id: string): boolean {
  return (
    id.startsWith(WORKFLOW_CONVERSATION_PREFIX) ||
    id.startsWith(PENDING_WORKFLOW_CONVERSATION_PREFIX)
  );
}

const WORKFLOW_SERVER_CONVERSATION_ID_PATTERN = /^chatc-[A-Za-z0-9_-]{1,120}$/;
const TYPED_SERVER_CONVERSATION_ID_PATTERN =
  /^nyxid-chat-[A-Za-z0-9_-]{1,117}$/;

const MAX_MESSAGE_CHARS = 32_768;

// Reference-client parity (nyxid-chat BFF `DEMO_STREAM_PROGRESS_TIMEOUT_MS`):
// a stream that produces only keepalives for this long is a hung run, not a
// slow one. Suspended while a human approval gate is open — external gates
// have no deadline the client can impose.
const STREAM_PROGRESS_TIMEOUT_MS = 120_000;

// How long a pre-RUN_STARTED cancel keeps the reader alive waiting for the
// frame that names the server turn. Without it the abort would discard the
// only chance to learn the turnId, leaving the upstream run uncancellable.
const PRE_START_STOP_WINDOW_MS = 5_000;

// Hard deadline on the `:stop` request itself. Without one, a server that
// accepts the connection but never answers would pin the `pendingStops`
// entry forever and tax every later send/delete with the full fence wait.
const STOP_REQUEST_DEADLINE_MS = 10_000;

// Hard deadline on the composite DELETE. The deletion reservation rejects
// sends and approvals while it holds, so an unanswered DELETE without a
// bound would lock the conversation permanently.
const DELETE_REQUEST_DEADLINE_MS = 15_000;

// Inline media larger than this (base64 chars ≈ 6 MB decoded) is summarized
// as text instead of being embedded as a data: URL artifact.
const MAX_MEDIA_DATA_CHARS = 8_000_000;

const MAX_TOOL_SUMMARY_CHARS = 160;

const HISTORY_RECONCILIATION_DELAYS_MS = [0, 300, 900, 1_800] as const;
const RESERVATION_RETRY_DELAYS_MS = [300, 900] as const;
const HISTORY_RESERVATION_UNAVAILABLE = "CHAT_HISTORY_RESERVATION_UNAVAILABLE";

// A POST can be retried only because `clientRequestId` makes Aevatar an
// idempotent receiver. Keep the budget deliberately small: one replay covers
// a dropped delivery without hiding a persistently unhealthy transport.
const STREAM_DELIVERY_ATTEMPTS = 2;
const RETRYABLE_STREAM_STATUSES = new Set([408, 425, 429, 500, 502, 503, 504]);

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

type AuthorizationReasonCode =
  | "NYXID_SERVICE_NOT_CONNECTED"
  | "NYXID_UNAUTHORIZED";

interface AuthorizationBlocker {
  readonly serviceSlug: string;
  readonly serviceLabel: string;
  readonly reasonCode: AuthorizationReasonCode;
  readonly safeMessage: string;
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
  readonly turnId?: string;
  readonly textMessageStart?: {
    readonly messageId?: string;
    readonly role?: string;
  };
  readonly textMessageContent?: { readonly delta?: string };
  readonly textMessageEnd?: { readonly messageId?: string };
  readonly toolCallStart?: ToolCallPayload;
  readonly toolCallEnd?: ToolCallPayload;
  readonly toolApprovalRequest?: ToolApprovalPayload;
  readonly authorizationRequired?: Record<string, unknown>;
  readonly usage?: UsagePayload;
  readonly mediaContent?: MediaPayload;
  readonly custom?: CustomEnvelope;
  readonly runStarted?: {
    readonly runId?: string;
    readonly turnId?: string;
    readonly actorId?: string;
  };
  readonly runFinished?: {
    readonly runId?: string;
    readonly status?: string;
    /** Workflow chat terminal payload (`WorkflowRunResultPayload`). */
    readonly result?: { readonly output?: string };
  };
  readonly runStopped?: { readonly reason?: string };
  readonly runError?: { readonly code?: string; readonly message?: string };
  readonly stepStarted?: { readonly stepName?: string };
  readonly stepFinished?: {
    readonly stepName?: string;
    readonly success?: boolean;
  };
  /** Workflow chat post-terminal projection snapshot; never rendered. */
  readonly stateSnapshot?: unknown;
}

/** `custom aevatar.chat.context` — first frame of a workflow chat turn. */
interface WorkflowChatContextPayload {
  readonly scopeId?: string;
  readonly conversationId?: string;
  readonly turnId?: string;
  /** Protobuf int64: rendered as a JSON string. */
  readonly stateVersion?: string | number;
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
  /** Observed as both ISO strings and epoch-ms numbers upstream. */
  readonly createdAt?: string | number;
  readonly updatedAt?: string | number;
  readonly messageCount?: number;
  readonly llmRoute?: string | null;
  readonly llmModel?: string | null;
}

/**
 * Normalize an index timestamp to an ISO string. The chat-history index
 * has been observed sending epoch-ms numbers (message timestamps in the
 * same API are epoch ms too); a number leaking into
 * `Conversation.last_message_at` crashes the sidebar sort's
 * `localeCompare` — which only fires once the list has 2+ rows, so a
 * single-conversation smoke test never sees it.
 */
function indexTimestampToIso(
  value: string | number | undefined,
  fallback: string,
): string {
  if (typeof value === "number" && Number.isFinite(value)) {
    return new Date(value).toISOString();
  }
  if (typeof value === "string" && value) return value;
  return fallback;
}

interface AevatarConversationListResponse {
  readonly conversations?: readonly AevatarHistoryIndexEntry[];
}

interface AevatarCreateRecoveryResponse {
  readonly status?: unknown;
  readonly conversationId?: unknown;
  readonly stateVersion?: unknown;
  readonly turnId?: unknown;
}

interface AevatarHistoryEntry {
  readonly id?: string;
  readonly role?: string;
  readonly content?: string | null;
  readonly timestamp?: number;
  readonly status?: unknown;
  readonly error?: unknown;
  readonly turnId?: unknown;
}

/**
 * Transcript read (`chat-history/conversations/{id}`), in both shapes we
 * accept.
 *
 * Aevatar PR #2923 wrapped the flat array in `{messages, stateVersion}`, where
 * `stateVersion` is the conversation read model's materialized watermark. The
 * legacy array stays accepted because NyxID and Aevatar deploy independently:
 * committing to one shape breaks chat either now or the moment Aevatar ships.
 *
 * REMOVE the array branch once every supported Aevatar environment is
 * confirmed on the wrapped contract — not after a single prod probe.
 *
 * `stateVersion` is the workflow-chat continuation watermark: continuing a
 * `chatc-…` conversation requires the last observed value as
 * `minimumStateVersion` (Aevatar's chat-history read fence). It is captured
 * when present and never required — legacy-array responses simply leave the
 * stored watermark unchanged.
 */
type AevatarHistoryResponse =
  | readonly AevatarHistoryEntry[]
  | { readonly messages?: unknown; readonly stateVersion?: unknown };

/** The wrapped shape's watermark, when the response carries a usable one. */
function historyStateVersion(body: AevatarHistoryResponse): number | undefined {
  if (Array.isArray(body)) return undefined;
  const raw = (body as { readonly stateVersion?: unknown }).stateVersion;
  return positiveStateVersion(raw);
}

function structuredBlockCount(messages: readonly AssistantMessage[]): number {
  return messages.reduce(
    (count, message) =>
      count + message.blocks.filter((block) => block.type !== "text").length,
    0,
  );
}

function hasStructuredBlocks(message: AssistantMessage): boolean {
  return message.blocks.some((block) => block.type !== "text");
}

/**
 * History v4 is text-only until `/state` card rehydration ships. Reinsert each
 * client-owned activity message after the last server row from its turn while
 * the server projection supplies text, ids, and turn status. This keeps typed
 * block ids/order stable without pinning stale text.
 */
function preserveLocalStructuredMessages(
  serverMessages: readonly AssistantMessage[],
  localMessages: readonly AssistantMessage[],
  activityMessageTurnIds: ReadonlyMap<string, string | null>,
): AssistantMessage[] {
  const buckets = Array.from(
    { length: serverMessages.length + 1 },
    () => [] as AssistantMessage[],
  );
  const serverMessageIds = new Set(serverMessages.map((message) => message.id));
  const lastServerIndexByTurnId = new Map<string, number>();
  serverMessages.forEach((message, index) => {
    if (message.turnId) lastServerIndexByTurnId.set(message.turnId, index);
  });

  for (const message of localMessages) {
    if (!hasStructuredBlocks(message)) continue;
    if (serverMessageIds.has(message.id)) continue;
    const turnId = activityMessageTurnIds.get(message.id);
    const serverIndex = turnId
      ? lastServerIndexByTurnId.get(turnId)
      : undefined;
    const insertionIndex =
      serverIndex === undefined ? serverMessages.length : serverIndex + 1;
    buckets[insertionIndex]?.push(message);
  }

  const merged: AssistantMessage[] = [...(buckets[0] ?? [])];
  serverMessages.forEach((message, index) => {
    merged.push(message, ...(buckets[index + 1] ?? []));
  });
  return merged;
}

interface StoredConversation {
  conversation: Conversation;
  turnState: TurnReducerState;
  /**
   * Stable hashed request identities keyed by actionRequestId. Valid requests
   * hash parsed params; recovered requests hash the original unparsed payload.
   * A fixed-size hash keeps the per-id retention bounded.
   */
  actionRequestFingerprints: Map<string, string>;
  /** Server turn ownership for client-only activity messages. */
  activityMessageTurnIds: Map<string, string | null>;
  /** Epoch milliseconds of the latest turn.completed applied to this mirror. */
  lastLocalTurnCompletedAt?: number;
  /**
   * Workflow-chat continuation watermark (`chatc-…` conversations only):
   * the last `stateVersion` observed from the transcript read or the
   * turn's `aevatar.chat.context` frame.
   */
  stateVersion?: number;
  /** A create command was dispatched but no canonical conversation is known. */
  identityPending?: boolean;
  /** A locally terminal workflow turn is not yet confirmed in wire history. */
  projectionPending?: boolean;
  /** Latest local workflow turn that must appear before materialization. */
  requiredTurnId?: string | null;
  /** Fallback fence when an interrupted delivery never announced a turn id. */
  requiredAssistantBaselineIds?: Set<string>;
  /** Reconciliation deadline, after which the mirror is explicitly stalled. */
  projectionStalledAt?: number;
  /** One-shot handoff from a cold transcript 404 to the reconciler. */
  lastWireObservationAt?: number;
  /**
   * Client session correlation handle sent on every workflow turn of this
   * conversation. Minted once and reused so the whole conversation shares
   * one session: a per-turn value would make the field meaningless, and
   * Aevatar folds `sessionId` into its create-replay fingerprint, so it must
   * also stay stable across a turn's delivery retries. Transport-local, so a
   * reload starts a new session id — nothing upstream reads it as identity
   * (conversation continuity is `conversationId` + `minimumStateVersion`).
   */
  sessionId?: string;
  /** Create idempotency identity retained across a user retry of one prompt. */
  createRequest?: { readonly prompt: string; readonly commandId: string };
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

interface PendingActionBatch {
  readonly id: string;
  readonly originTurnId: string;
  /** NyxId chat actor that owns the origin turn. */
  readonly actorId: string | null;
  readonly reports: Map<string, ActionReport>;
  onEvent: (event: TurnEvent) => void;
  clientRequestId: string | null;
  inFlight: boolean;
  blocked: boolean;
}

interface ActionContinuationState {
  readonly batchId: string;
  retryQueued: boolean;
  /** True once the batch was explicitly accepted or explicitly requeued. */
  settled: boolean;
}

interface RunningTurn {
  readonly clientRequestId: string;
  /**
   * Which turn surface this run speaks: `workflow` = the studio workflow
   * chat (every new conversation), `actor` = an existing nyxid-chat
   * conversation on the typed NyxIdChat surface. Chosen from the
   * conversation-id family at send time; gates the frame-ordering rules
   * (`aevatar.chat.context` precedes `runStarted` on workflow streams,
   * trailing `stateSnapshot` follows the terminal) and disables the
   * `:stop` control, which only the actor surface serves.
   */
  readonly protocol: "actor" | "workflow";
  turnId: string | null;
  /**
   * A cancel landed before RUN_STARTED delivered the server turn id. The
   * reader stays alive (bounded) so the announcing frame can still arrive;
   * the RUN_STARTED handler then submits the stop and aborts.
   */
  stopPendingStart: boolean;
  /**
   * The stream (or continuation) fetch actually left the client. A cancel
   * before dispatch is purely local: nothing reached upstream, so there is
   * no turn to stop — and, critically, no pre-start placeholder may be
   * installed, or it would overwrite an earlier turn's still-pending stop
   * fence and let a later send overtake it.
   */
  streamDispatched: boolean;
  /**
   * Lifts the placeholder fence a pre-start cancel installed in
   * `pendingStops` — called once the deferred stop settles, or when the
   * pre-start window expires without a turn to stop.
   */
  resolvePreStartFence?: () => void;
  turnAnnounced: boolean;
  readonly controller: AbortController;
  readonly onEvent: (event: TurnEvent) => void;
  cursor: number;
  currentMessageId: string | null;
  currentBlockId: string | null;
  accumulatedText: string;
  finished: boolean;
  /** A turn event met the event pump's exact printable-content contract. */
  assistantContentObserved: boolean;
  /** Whether a later server-side assistant row can still materialize. */
  serverAnswerExpectation: "possible" | "none";
  /** Assistant rows already present before this stream was dispatched. */
  assistantMessageIdsAtDispatch: Set<string>;
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
  /** Service dedupe key → connect card block_id (for in-place upgrades). */
  promptedConnectSlugs: Map<string, string>;
  /** Action request id → action card block id. */
  promptedActionIds: Map<string, string>;
  waitingForApproval: boolean;
  watchdog: ReturnType<typeof setTimeout> | null;
  deliveryStarted: boolean;
  deliveryTerminal:
    | { readonly kind: "finished"; readonly status: "completed" | "blocked" }
    | {
        readonly kind: "error";
        readonly error: { readonly code: string; readonly message: string };
      }
    | { readonly kind: "stopped" }
    | null;
  deliveryTerminalCount: number;
  deliveryProtocolError: {
    readonly code: string;
    readonly message: string;
  } | null;
  actionContinuation: ActionContinuationState | null;
  optimisticMessageAppended: boolean;
  createRecoveryStarted: boolean;
  /** History recovery already proved the exact assistant turn exists. */
  createRecoveryMaterialized: boolean;
  activeWireTelemetry: StreamWireTelemetry | null;
}

type ReconcileOrigin =
  | "post_terminal"
  | "cold_observed"
  | "identity_recovery"
  | "explicit_retry";

interface ReconcileEntry {
  readonly promise: Promise<ProjectionReconcileOutcome>;
  readonly settle: (outcome: ProjectionReconcileOutcome) => void;
  readonly scopeId: string | null;
  readonly placeholderId?: string;
  conversationId: string;
  commandId?: string;
  readonly origin: ReconcileOrigin;
  attempt: number;
  startedAt: number;
  deadlineAt: number;
  nextAttemptAt?: number;
  waiters: number;
  timer?: ReturnType<typeof setTimeout>;
  controller?: AbortController;
  running: boolean;
  finalObservationDue: boolean;
  firstAbsentAt?: number;
}

interface StreamWireTelemetry {
  exchangeId?: string | null;
  readonly startedAt: number;
  framesSeen: number;
  printableFramesSeen: number;
  printableTurnEvents: number;
  wireBytes: number;
  terminalReceived: boolean;
  firstFrameAt?: number;
  lastFrameAt?: number;
  transportOutcome?: string;
}

type StreamConsumptionResult =
  | { readonly kind: "settled" }
  | {
      readonly kind: "retryable";
      readonly error: { code: string; message: string };
    }
  | {
      readonly kind: "protocol_error";
      readonly error: { code: string; message: string };
    };

function newId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

function noopEvent(): void {
  // Continuation stream with no subscriber: state still lands in the
  // transport mirror via `emit`.
}

function turnEventPrintsAssistantContent(event: TurnEvent): boolean {
  switch (event.event) {
    case "block.delta":
      return event.text.trim().length > 0;
    case "block.updated":
      return "decision" in event.patch && event.patch.decision !== null;
    case "block.started":
    case "block.completed":
      return (
        event.block.type !== "text" ||
        (typeof event.block.text === "string" &&
          event.block.text.trim().length > 0)
      );
    default:
      return false;
  }
}

function isoFromEpochMs(epochMs: number | undefined, fallback: string): string {
  if (typeof epochMs !== "number" || !Number.isFinite(epochMs)) {
    return fallback;
  }
  return new Date(epochMs).toISOString();
}

const OPAQUE_TURN_ID_PATTERN = /^[A-Za-z0-9._:-]{1,256}$/;

function safeTurnId(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return OPAQUE_TURN_ID_PATTERN.test(normalized) ? normalized : null;
}

function positiveStateVersion(value: unknown): number | undefined {
  const parsed =
    typeof value === "number"
      ? value
      : typeof value === "string"
        ? Number(value)
        : NaN;
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : undefined;
}

function abortableDelay(delayMs: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) {
    return Promise.reject(new DOMException("Aborted", "AbortError"));
  }
  if (delayMs <= 0) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timeout = globalThis.setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, delayMs);
    const onAbort = () => {
      globalThis.clearTimeout(timeout);
      reject(new DOMException("Aborted", "AbortError"));
    };
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

function historyIncludesAssistantTurn(
  entries: readonly AevatarHistoryEntry[],
  turnId: string | null,
): boolean {
  if (!turnId) return true;
  return entries.some(
    (entry) =>
      entry.role === "assistant" && safeTurnId(entry.turnId) === turnId,
  );
}

function historyIncludesNewAssistantMessage(
  entries: readonly AevatarHistoryEntry[],
  baselineIds: ReadonlySet<string>,
): boolean {
  return entries.some(
    (entry) =>
      entry.role?.toLowerCase() === "assistant" &&
      typeof entry.id === "string" &&
      !baselineIds.has(entry.id),
  );
}

function safeErrorCode(value: unknown, fallback: string): string {
  return typeof value === "string" && /^[A-Za-z0-9._:-]{1,128}$/.test(value)
    ? value
    : fallback;
}

function safeErrorMessage(value: unknown, fallback: string): string {
  if (typeof value !== "string" || !value.trim()) return fallback;
  return redactDisplayText(value.trim()).slice(0, 1_024);
}

function historyStatus(value: unknown): TurnStatus | undefined {
  switch (value) {
    case "running":
    case "waiting":
    case "blocked":
    case "completed":
    case "failed":
    case "cancelled":
      return value;
    default:
      return undefined;
  }
}

function historyError(
  value: unknown,
):
  | string
  | { readonly code: string; readonly message: string }
  | null
  | undefined {
  if (value === null) return null;
  if (typeof value === "string") {
    return safeErrorMessage(value, "The assistant turn failed.");
  }
  if (typeof value !== "object" || value === null) return undefined;
  const record = value as Record<string, unknown>;
  if (typeof record["message"] !== "string") return undefined;
  return {
    code: safeErrorCode(record["code"], "history_error"),
    message: safeErrorMessage(record["message"], "The assistant turn failed."),
  };
}

function textToBlocks(
  text: string,
  messageId: string,
): readonly ContentBlock[] {
  return [{ type: "text", block_id: `${messageId}-text`, text }];
}

function historyEntryToMessage(
  entry: AevatarHistoryEntry,
  index: number,
): AssistantMessage | null {
  if (entry.role !== "user" && entry.role !== "assistant") return null;
  const id = entry.id ?? `history-${String(index)}`;
  const text = entry.content ?? "";
  const status = historyStatus(entry.status);
  const error = historyError(entry.error);
  const turnId = safeTurnId(entry.turnId);
  return {
    id,
    role: entry.role,
    schema_version: 1,
    blocks: text ? [...textToBlocks(text, id)] : [],
    created_at: isoFromEpochMs(entry.timestamp, new Date(0).toISOString()),
    ...(turnId ? { turnId } : {}),
    ...(status ? { status } : {}),
    ...(error !== undefined ? { error } : {}),
  };
}

/**
 * Narrow a transcript read to its message entries, or reject it.
 *
 * Strict on purpose. A permissive reader that degraded `{}` or
 * `{messages: null}` to an empty transcript would render "no messages" for a
 * broken upstream — the same silent failure the `stateVersion` wrapper caused
 * against the old array-typed reader. An empty array (or an empty wrapped
 * `messages`) remains a valid answer meaning deleted / not yet materialized /
 * zero turns; only a body that is neither accepted shape is a protocol error.
 */
function readHistoryEntries(
  body: AevatarHistoryResponse,
): readonly AevatarHistoryEntry[] {
  if (Array.isArray(body)) return body;
  if (body && typeof body === "object") {
    const { messages } = body as { readonly messages?: unknown };
    if (Array.isArray(messages)) {
      return messages as readonly AevatarHistoryEntry[];
    }
  }
  throw new AssistantProtocolError(
    "The conversation history response did not match the expected shape.",
  );
}

function deriveTitle(messages: AssistantMessage[]): string | null {
  const firstUser = messages.find((message) => message.role === "user");
  const firstText = firstUser?.blocks.find((block) => block.type === "text");
  if (!firstText || firstText.type !== "text" || !firstText.text.trim()) {
    return null;
  }
  return firstText.text.trim().slice(0, 40);
}

function stableJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => stableJsonValue(entry));
  }
  if (value && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).sort(
      ([left], [right]) => (left < right ? -1 : left > right ? 1 : 0),
    );
    return Object.fromEntries(
      entries.map(([key, entry]) => [key, stableJsonValue(entry)]),
    );
  }
  return value;
}

function stableJsonText(value: unknown): string {
  const serialized = JSON.stringify(stableJsonValue(value));
  return serialized ?? "undefined";
}

// Non-crypto fingerprint: enough to spot same-id request drift while keeping
// the retained per-request footprint fixed.
function fnv1aHex(text: string): string {
  let hash = 0x811c9dc5;
  for (const char of text) {
    hash ^= char.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}

function fingerprintStableRequestInput(
  value: AssistantActionRequest["params"] | unknown,
): string {
  return fnv1aHex(stableJsonText(value));
}

function sameStringArray(
  left: readonly string[],
  right: readonly string[],
): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function sameActionCardParams(
  left: ActionCardContentBlock["params"],
  right: ActionCardContentBlock["params"],
): boolean {
  if (left.variant !== right.variant) return false;
  switch (left.variant) {
    case "catalog":
      return (
        right.variant === "catalog" &&
        left.service_slug === right.service_slug &&
        sameStringArray(left.requested_scopes, right.requested_scopes) &&
        left.via_node_id === right.via_node_id &&
        left.target_org_id === right.target_org_id
      );
    case "custom":
      return (
        right.variant === "custom" &&
        left.name === right.name &&
        left.endpoint_url === right.endpoint_url &&
        left.auth_method === right.auth_method &&
        left.auth_key_name === right.auth_key_name &&
        left.via_node_id === right.via_node_id &&
        left.target_org_id === right.target_org_id
      );
    case "unknown":
      return right.variant === "unknown";
  }
}

function matchesCommittedActionRequest(
  block: ActionCardContentBlock,
  request: AssistantActionRequest,
  params: ActionCardContentBlock["params"],
  committedFingerprint: string | undefined,
  requestFingerprint: string,
): boolean {
  return (
    block.action === request.action &&
    block.origin_turn_id === request.originTurnId &&
    (block.actor_id ?? "") === request.actorId &&
    block.task_id === request.taskId &&
    block.step_id === request.stepId &&
    sameActionCardParams(block.params, params) &&
    committedFingerprint === requestFingerprint
  );
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
    // Envelope messages render in toasts and the thread; redact like any
    // other upstream-derived display string.
    message: envelopeMessage
      ? redactDisplayText(envelopeMessage)
      : "The assistant stream could not be started.",
  };
}

function isRetryableHistoryRefreshError(error: unknown): boolean {
  // A tombstone means the local conversation was deleted while refresh was in
  // flight. Unlike the console's generic status-less failures, retrying cannot
  // make that conversation valid again, so preserve the not-found result.
  if (error instanceof AssistantConversationNotFoundError) return false;
  if (error instanceof AssistantProtocolError) return false;
  if (error instanceof ApiError) {
    return error.status >= 500 && error.status < 600;
  }
  // Raw fetch/network failures carry no HTTP status and are transient in the
  // console retry contract, so they consume the next reservation attempt.
  return true;
}

// ---------------------------------------------------------------------------
// Redaction and tool-result summaries (ported from the reference client's
// `protocol.js`): display strings derived from tool payloads are untrusted
// and may echo credentials; PRD §3.6 forbids raw downstream bodies in blocks.
// ---------------------------------------------------------------------------

// Auth-scheme values plus BARE well-known token shapes: provider keys leak
// into natural-language error strings ("AWS rejected AKIA...") where no
// key=value assignment exists for the assignment rule to catch.
const SECRET_VALUE_PATTERN =
  /((?:Bearer|Basic)\s+)[A-Za-z0-9._~+/=-]+|\beyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b|nyx(?:id)?_[A-Za-z0-9_-]{8,}|\b(?:AKIA|ASIA|ABIA|ACCA)[A-Z0-9]{16}\b|\bsk[-_][A-Za-z0-9_-]{8,}\b|\bgh[pousr]_[A-Za-z0-9]{20,}\b|\bAIza[A-Za-z0-9_-]{30,}\b|\bxox[baprs]-[A-Za-z0-9-]{10,}\b/gi;

// Key names match with prefixes/suffixes (`secretAccessKey`, `x-api-key`,
// `authorizationHeader`); quoted values — double or single (Python-style
// reprs) — are consumed whole so a secret containing spaces cannot leak
// its tail.
const SECRET_ASSIGNMENT_PATTERN =
  /(["']?[\w.-]*(?:authorization|api[-_]?key|access[-_]?key[-_]?id|token|secret|password|credential|cookie)[\w.-]*["']?\s*[:=]\s*)("[^"]*"|'[^']*'|[^",'\s}]+)/gi;

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

/**
 * Accept only the published NyxID blocker contract. Tool prose, generic
 * authorization frames, and unknown reason codes remain ordinary failures.
 */
function parseAuthorizationBlocker(
  payload: unknown,
): AuthorizationBlocker | null {
  const record = unpackAny(payload);
  const reasonCode = record["reasonCode"];
  if (
    reasonCode !== "NYXID_SERVICE_NOT_CONNECTED" &&
    reasonCode !== "NYXID_UNAUTHORIZED"
  ) {
    return null;
  }
  const rawSlug = record["serviceSlug"];
  if (typeof rawSlug !== "string") return null;
  const serviceSlug = rawSlug.trim().toLowerCase();
  if (!/^[a-z0-9][a-z0-9-]{0,127}$/.test(serviceSlug)) return null;

  const rawLabel = record["serviceLabel"];
  const rawMessage = record["safeMessage"];
  const serviceLabel =
    typeof rawLabel === "string" && rawLabel.trim()
      ? safeErrorMessage(rawLabel, humanizeSlug(serviceSlug)).slice(0, 128)
      : humanizeSlug(serviceSlug);
  const safeMessage =
    typeof rawMessage === "string" && rawMessage.trim()
      ? safeErrorMessage(
          rawMessage,
          `Connect or reauthorize ${serviceSlug} to continue.`,
        )
      : `Connect or reauthorize ${serviceSlug} to continue.`;

  return { serviceSlug, serviceLabel, reasonCode, safeMessage };
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
  private readonly pendingActionBatches = new Map<
    string,
    PendingActionBatch[]
  >();
  private readonly actionDrainBlocked = new Set<string>();
  /**
   * In-flight `:stop` fence per conversation. Follow-up sends and the
   * composite delete serialize behind it so a fast next action cannot
   * reach Aevatar before the stop fence commits. Every entry is
   * self-bounded (stop deadline / pre-start window), so waiters await it
   * directly.
   */
  private readonly pendingStops = new Map<string, Promise<void>>();
  /**
   * Tombstones for server-accepted deletes: the Chat History index is
   * eventually consistent, so stale list/history responses can still carry
   * a row we just deleted. Tombstones are PERMANENT for the transport's
   * lifetime — actor ids are never reused, and retiring one early would
   * reopen the race where an in-flight pre-delete read lands after the
   * retire and resurrects the conversation.
   */
  private readonly deletedConversationIds = new Set<string>();
  /**
   * Deletion-in-progress reservation: the one in-flight delete operation
   * per conversation, installed before the delete's cancel and fence wait
   * and cleared when it settles (the tombstone takes over on success).
   * Sends and approvals are rejected while present — otherwise a
   * successor turn admitted during the fence wait can dispatch its stream
   * before the DELETE and recreate the actor the user asked to remove.
   * Concurrent deletes coalesce onto the stored promise.
   */
  private readonly deletingConversations = new Map<string, Promise<void>>();
  /**
   * Placeholder → server id, written when a create-and-first-turn stream
   * identifies its authoritative conversation. Public methods resolve
   * through this so a page still addressing the local placeholder reaches
   * the server-backed entry.
   */
  private readonly conversationAliases = new Map<string, string>();
  /** Conversation currently materialized in the chat view. */
  private activeConversationId: string | null = null;
  private readonly now: () => number;
  private readonly random: () => number;
  private ownerScopeId: string | null;
  private readonly scopeControllers = new Set<AbortController>();
  private readonly reconcileEntries = new Map<string, ReconcileEntry>();
  private readonly streamWireTelemetry = new WeakMap<
    ChatStreamRequestHandle,
    StreamWireTelemetry
  >();
  private readonly deletionCleanup = new Map<string, Promise<void>>();
  private deletionSweepStarted = false;
  private listFetchedAt = 0;

  constructor(
    now: () => number = Date.now,
    random: () => number = Math.random,
  ) {
    this.now = now;
    this.random = random;
    this.ownerScopeId = useAuthStore.getState().user?.id ?? null;
    useAuthStore.subscribe((state, previousState) => {
      const nextScope = state.user?.id ?? null;
      if (nextScope !== (previousState.user?.id ?? null)) {
        this.resetScope(nextScope);
      }
    });
  }

  private ensureScope(): string | null {
    const nextScope = useAuthStore.getState().user?.id ?? null;
    if (nextScope !== this.ownerScopeId) this.resetScope(nextScope);
    return this.ownerScopeId;
  }

  private resetScope(nextScope: string | null): void {
    for (const run of this.running.values()) {
      run.controller.abort();
      this.clearWatchdog(run);
    }
    for (const controller of this.scopeControllers) controller.abort();
    for (const entry of [...this.reconcileEntries.values()]) {
      if (entry.timer !== undefined) clearTimeout(entry.timer);
      entry.controller?.abort();
      entry.settle({
        status: "timed_out",
        conversationId: entry.conversationId,
      });
    }
    this.conversations.clear();
    this.running.clear();
    this.pendingStops.clear();
    this.pendingActionBatches.clear();
    this.actionDrainBlocked.clear();
    this.deletedConversationIds.clear();
    this.deletingConversations.clear();
    this.conversationAliases.clear();
    this.scopeControllers.clear();
    this.reconcileEntries.clear();
    this.deletionCleanup.clear();
    this.activeConversationId = null;
    this.listFetchedAt = 0;
    this.deletionSweepStarted = false;
    this.ownerScopeId = nextScope;
  }

  private scopeController(): AbortController {
    const controller = new AbortController();
    this.scopeControllers.add(controller);
    controller.signal.addEventListener(
      "abort",
      () => this.scopeControllers.delete(controller),
      { once: true },
    );
    return controller;
  }

  private releaseScopeController(controller: AbortController): void {
    this.scopeControllers.delete(controller);
  }

  /** Follow a placeholder alias to the server conversation id, if any. */
  private canonicalConversationId(conversationId: string): string {
    return this.conversationAliases.get(conversationId) ?? conversationId;
  }

  async listConversations(): Promise<Conversation[]> {
    const scopeId = this.ensureScope();
    if (!this.deletionSweepStarted) {
      this.deletionSweepStarted = true;
      void this.sweepDeletionIntents(scopeId);
    }
    const now = Date.now();
    if (
      this.running.size === 0 &&
      now - this.listFetchedAt > CONVERSATION_LIST_TTL_MS
    ) {
      this.listFetchedAt = now;
      const response = await assistantApi.get<AevatarConversationListResponse>(
        `${ASSISTANT_PREFIX}/conversations`,
      );
      if (scopeId !== this.ensureScope()) return [];
      const deletionIntentIds = new Set(
        listDeletionIntents().flatMap((intent) =>
          intent.conversationId ? [intent.conversationId] : [],
        ),
      );
      for (const entry of response.conversations ?? []) {
        const id = entry?.id?.trim();
        if (id) this.mergeIndexEntry(id, entry, deletionIntentIds);
      }
    }
    // An aliased conversation is stored under both its placeholder key and
    // its server id (`conversation.id` is the server id for both), so the
    // list dedupes by id — preferring the entry stored under its own id,
    // which is the one `mergeIndexEntry`/`loadHistory` keep fresh.
    const byId = new Map<string, Conversation>();
    for (const [key, stored] of this.conversations) {
      const conversation = stored.conversation;
      if (!byId.has(conversation.id) || key === conversation.id) {
        byId.set(conversation.id, conversation);
      }
    }
    return [...byId.values()].sort((a, b) =>
      // Timestamps are normalized to ISO strings at the index boundary;
      // String() keeps a stray non-string from crashing the whole list
      // (ISO strings order identically either way).
      String(b.last_message_at).localeCompare(String(a.last_message_at)),
    );
  }

  async createConversation(): Promise<Conversation> {
    this.ensureScope();
    // New chats run on the workflow (studio) surface, where the conversation
    // is created server-side by the FIRST turn's chat-history reservation
    // (`conversation.conversationId: null` in the turn body) — there is no
    // separate create call. Until that turn's `aevatar.chat.context` frame
    // delivers the server `chatc-…` id, the conversation exists only under
    // this client-local placeholder id; the frame aliases it in place.
    const createdAt = new Date().toISOString();
    const conversation: Conversation = {
      id: `${PENDING_WORKFLOW_CONVERSATION_PREFIX}${crypto.randomUUID()}`,
      title: "New chat",
      created_at: createdAt,
      last_message_at: createdAt,
    };
    this.conversations.set(conversation.id, {
      conversation,
      turnState: EMPTY_TURN_STATE,
      actionRequestFingerprints: new Map(),
      activityMessageTurnIds: new Map(),
    });
    this.activeConversationId = conversation.id;
    return conversation;
  }

  async deleteConversation(conversationId: string): Promise<void> {
    this.ensureScope();
    const requestedId = conversationId;
    const pendingReceipt = conversationId.startsWith(
      PENDING_WORKFLOW_CONVERSATION_PREFIX,
    )
      ? findReceiptByPlaceholder(conversationId)
      : undefined;
    const aliasedConversationId = this.conversationAliases.get(conversationId);
    if (
      conversationId.startsWith(PENDING_WORKFLOW_CONVERSATION_PREFIX) &&
      !pendingReceipt?.conversationId &&
      !aliasedConversationId
    ) {
      const run = this.running.get(conversationId);
      if (run) this.cancelTurn(conversationId, run);
      if (pendingReceipt) {
        recordDeletionIntent(
          pendingReceipt.commandId,
          pendingReceipt.placeholderId,
          undefined,
          this.now(),
        );
        deleteReceipt(pendingReceipt.commandId);
      }
      this.tombstoneConversation(conversationId);
      if (pendingReceipt) {
        void this.startDeletionIntentCleanup(
          pendingReceipt.commandId,
          this.ownerScopeId,
        );
      }
      return;
    }
    if (pendingReceipt?.conversationId) {
      this.conversationAliases.set(
        conversationId,
        pendingReceipt.conversationId,
      );
    }
    conversationId = this.canonicalConversationId(conversationId);
    // A turn still streaming into the conversation must not keep painting
    // a transcript that is about to disappear: cancel first, matching the
    // reference client's abort-then-delete order. The server side is the
    // #1199 composite delete (actor + history row, 404-tolerant), so one
    // call retires both upstream surfaces.
    //
    // Reserve the conversation for the whole delete: the cancel below emits
    // the terminal synchronously, so without the reservation a successor
    // send admitted during the fence wait could dispatch its stream before
    // the DELETE and recreate the actor the user asked to remove.
    // Concurrent deletes COALESCE onto the one in-flight operation — a
    // flag-style reservation is not ownership-safe (an overlapping call's
    // failure would clear it while the other DELETE is still in flight).
    const inFlight = this.deletingConversations.get(conversationId);
    if (inFlight) return inFlight;
    // The body is DEFERRED to a microtask so the reservation is installed
    // before any callback-capable work runs: cancelTurn emits
    // `turn.completed` synchronously, and a re-entrant callback must
    // already see the deletion guard — otherwise it can admit a send (or
    // a second delete) into the exact window the reservation closes.
    const operation = Promise.resolve().then(async () => {
      // A create-and-first-turn run is keyed under the placeholder id the
      // send used; the same conversation is addressable through both.
      for (const key of [requestedId, conversationId]) {
        const run = this.running.get(key);
        if (run) {
          this.cancelTurn(key, run);
          break;
        }
      }
      // The cancel above may have fired a `:stop`; let its fence commit
      // before the actor delete races the still-active work upstream.
      await this.awaitPendingStop(conversationId);
      // Own deadline: the reservation rejects sends while it holds, so an
      // accepted-but-never-answered DELETE must not lock the conversation.
      const deadline = this.scopeController();
      const deadlineTimer = setTimeout(
        () => deadline.abort(),
        DELETE_REQUEST_DEADLINE_MS,
      );
      try {
        await apiClient<unknown>(
          `${ASSISTANT_PREFIX}/conversations/${conversationId}`,
          {
            method: "DELETE",
            preserveSessionOn401: true,
            signal: deadline.signal,
            ...assistantWireLogOptions(),
          },
        );
      } finally {
        clearTimeout(deadlineTimer);
        this.releaseScopeController(deadline);
      }
      // Local removal only after the server accepted: a failed delete keeps
      // the conversation listed and retryable. A placeholder that aliased
      // to this conversation is tombstoned with it, so neither address can
      // resurrect the row.
      this.conversations.delete(conversationId);
      this.deletedConversationIds.add(conversationId);
      this.pendingActionBatches.delete(conversationId);
      this.actionDrainBlocked.delete(conversationId);
      const receipt = findReceiptByConversation(conversationId);
      if (receipt) deleteReceipt(receipt.commandId);
      for (const [placeholder, target] of this.conversationAliases) {
        if (target === conversationId) {
          this.conversations.delete(placeholder);
          this.deletedConversationIds.add(placeholder);
          this.pendingActionBatches.delete(placeholder);
          this.actionDrainBlocked.delete(placeholder);
        }
      }
    });
    this.deletingConversations.set(conversationId, operation);
    try {
      await operation;
    } finally {
      // On success the tombstone takes over; on failure the conversation
      // becomes usable (and retryable) again. Identity-checked: only the
      // entry this owner installed is cleared.
      if (this.deletingConversations.get(conversationId) === operation) {
        this.deletingConversations.delete(conversationId);
      }
    }
  }

  private async sweepDeletionIntents(scopeId: string | null): Promise<void> {
    await Promise.allSettled(
      listDeletionIntents().map((intent) =>
        this.startDeletionIntentCleanup(intent.commandId, scopeId),
      ),
    );
  }

  private startDeletionIntentCleanup(
    commandId: string,
    scopeId: string | null,
  ): Promise<void> {
    const existing = this.deletionCleanup.get(commandId);
    if (existing) return existing;
    const operation = this.cleanupDeletionIntent(commandId, scopeId).finally(
      () => {
        if (this.deletionCleanup.get(commandId) === operation) {
          this.deletionCleanup.delete(commandId);
        }
      },
    );
    this.deletionCleanup.set(commandId, operation);
    return operation;
  }

  private async cleanupDeletionIntent(
    commandId: string,
    scopeId: string | null,
  ): Promise<void> {
    if (!scopeId || scopeId !== this.ownerScopeId) return;
    let intent = listDeletionIntents().find(
      (candidate) => candidate.commandId === commandId,
    );
    if (!intent) return;
    const controller = this.scopeController();
    const deadlineAt = this.now() + CREATE_RECOVERY_BACKOFF_POLICY.deadlineMs;
    try {
      let conversationId = intent.conversationId;
      for (let attempt = 0; !conversationId; attempt += 1) {
        if (controller.signal.aborted || scopeId !== this.ownerScopeId) return;
        try {
          const response =
            await assistantApi.get<AevatarCreateRecoveryResponse>(
              `${ASSISTANT_PREFIX}/conversations/create-recovery/${encodeURIComponent(commandId)}`,
              controller.signal,
            );
          const recovered = this.decodeCreateRecovery(response);
          conversationId = recovered.conversationId;
          recordDeletionIntent(
            commandId,
            intent.placeholderId,
            conversationId,
            intent.createdAt,
          );
          intent = { ...intent, conversationId };
        } catch (error) {
          if (controller.signal.aborted || scopeId !== this.ownerScopeId)
            return;
          if (!(error instanceof ApiError && error.status === 404)) return;
          if (this.now() >= deadlineAt) return;
          await abortableDelay(
            nextBackoffDelay(
              CREATE_RECOVERY_BACKOFF_POLICY,
              attempt,
              this.random,
            ),
            controller.signal,
          );
        }
      }
      if (!conversationId || scopeId !== this.ownerScopeId) return;
      const deleteTimer = setTimeout(
        () => controller.abort(),
        DELETE_REQUEST_DEADLINE_MS,
      );
      try {
        await apiClient<unknown>(
          `${ASSISTANT_PREFIX}/conversations/${conversationId}`,
          {
            method: "DELETE",
            preserveSessionOn401: true,
            signal: controller.signal,
            ...assistantWireLogOptions(),
          },
        );
      } finally {
        clearTimeout(deleteTimer);
      }
      if (scopeId !== this.ownerScopeId) return;
      this.tombstoneConversation(intent.placeholderId);
      this.tombstoneConversation(conversationId);
      resolveDeletionIntent(commandId);
    } catch {
      // The persisted intent is retried on the next per-scope sweep.
    } finally {
      controller.abort();
      this.releaseScopeController(controller);
    }
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    const scopeId = this.ensureScope();
    const requestedId = conversationId;
    if (
      conversationId.startsWith(PENDING_WORKFLOW_CONVERSATION_PREFIX) &&
      !this.conversations.has(conversationId)
    ) {
      const receipt = findReceiptByPlaceholder(conversationId);
      if (!receipt) throw new AssistantConversationNotFoundError();
      if (receipt.conversationId) {
        this.conversationAliases.set(conversationId, receipt.conversationId);
        conversationId = receipt.conversationId;
      } else {
        const stored = this.syntheticPendingConversation(conversationId, {
          identityPending: true,
          stateVersion: receipt.stateVersion,
        });
        this.conversations.set(conversationId, stored);
      }
    }
    conversationId = this.canonicalConversationId(conversationId);
    if (this.deletedConversationIds.has(conversationId)) {
      throw new AssistantConversationNotFoundError();
    }
    let existing = this.conversations.get(conversationId);
    const canonicalReceipt = findReceiptByConversation(conversationId);
    if (!existing && canonicalReceipt) {
      existing = this.syntheticPendingConversation(conversationId, {
        projectionPending: true,
        stateVersion: canonicalReceipt.stateVersion,
      });
      this.conversations.set(conversationId, existing);
      if (requestedId !== conversationId) {
        this.conversations.set(requestedId, existing);
      }
    }
    if (
      !existing &&
      (conversationId.startsWith(LEGACY_PENDING_TYPED_CONVERSATION_PREFIX) ||
        conversationId.startsWith(PENDING_WORKFLOW_CONVERSATION_PREFIX))
    ) {
      throw new AssistantConversationNotFoundError();
    }
    const turnInFlight =
      this.running.has(requestedId) || this.running.has(conversationId);
    // During a streaming turn the local mirror is ahead of the server;
    // serving it keeps per-event re-projection off the network entirely.
    if (
      existing &&
      (turnInFlight || isTurnActive(existing.turnState.activeTurn?.status))
    ) {
      this.activateConversation(conversationId, existing);
      return this.historyFromStored(existing, true);
    }
    // Only an ACTIVE reconciliation owns the wire: while it polls, every
    // other read serves the mirror. A STALLED record (the reconciler's
    // bounded wait expired) must not — the deadline ends aggressive polling,
    // not observation. Falling through here makes every later read (mount,
    // window focus, list navigation) one low-frequency wire observation, so
    // a reply that materializes after the deadline still renders; a 404
    // keeps serving the stalled mirror via the fallback below.
    if (
      existing &&
      (existing.identityPending ||
        (existing.projectionPending &&
          existing.projectionStalledAt === undefined &&
          (existing.turnState.messages.length > 0 ||
            existing.requiredTurnId != null ||
            existing.lastLocalTurnCompletedAt !== undefined)))
    ) {
      this.activateConversation(conversationId, existing);
      return this.historyFromStored(existing);
    }
    // A pending placeholder exists nowhere server-side — the id never left
    // this client — so the read can only 404 and land in the
    // `noServerTranscriptYet` fallback below. Serve the local mirror without
    // the round trip.
    if (
      existing &&
      (conversationId.startsWith(LEGACY_PENDING_TYPED_CONVERSATION_PREFIX) ||
        conversationId.startsWith(PENDING_WORKFLOW_CONVERSATION_PREFIX))
    ) {
      this.activateConversation(conversationId, existing);
      return this.historyFromStored(existing);
    }
    let stored: StoredConversation;
    try {
      stored = await this.loadHistory(conversationId);
    } catch (error) {
      // Delete wins over every other outcome, including the throws below:
      // once the conversation is tombstoned, "not found" is the answer, not
      // whatever the doomed read happened to fail with.
      if (this.deletedConversationIds.has(conversationId)) {
        throw new AssistantConversationNotFoundError();
      }
      // A contract break must reach the user. Swallowing it here is what
      // turned the PR #2923 array→`{messages, stateVersion}` change into a
      // blank transcript instead of a visible failure.
      if (error instanceof AssistantProtocolError) throw error;
      // Nothing local to answer with. A server 404 confirms the id is unknown;
      // transient and protocol failures retain their original type.
      if (!existing) {
        if (error instanceof ApiError && error.status === 404) {
          if (!conversationId.startsWith(WORKFLOW_CONVERSATION_PREFIX)) {
            throw new AssistantConversationNotFoundError();
          }
          const membership = await this.fetchRawIndexMembership(conversationId);
          if (scopeId !== this.ensureScope()) {
            throw new AssistantConversationNotFoundError();
          }
          if (membership === "unavailable" && !canonicalReceipt) throw error;
          if (membership !== true && !canonicalReceipt) {
            throw new AssistantConversationNotFoundError();
          }
          stored =
            this.conversations.get(conversationId) ??
            this.syntheticPendingConversation(conversationId, {
              projectionPending: true,
              stateVersion: canonicalReceipt?.stateVersion,
            });
          stored.projectionPending = true;
          stored.stateVersion =
            canonicalReceipt?.stateVersion ?? stored.stateVersion;
          stored.lastWireObservationAt = this.now();
          this.conversations.set(conversationId, stored);
        } else {
          throw error;
        }
      } else {
        const noServerTranscriptYet =
          error instanceof ApiError && error.status === 404;
        if (
          noServerTranscriptYet &&
          conversationId.startsWith(WORKFLOW_CONVERSATION_PREFIX) &&
          existing.turnState.messages.length === 0 &&
          !existing.identityPending &&
          !existing.projectionPending
        ) {
          const membership = await this.fetchRawIndexMembership(conversationId);
          if (scopeId !== this.ensureScope() || membership === false) {
            this.tombstoneConversation(conversationId);
            throw new AssistantConversationNotFoundError();
          }
          if (membership === true) existing.projectionPending = true;
          if (membership === "unavailable") throw error;
          existing.lastWireObservationAt = this.now();
        }
        if (
          !noServerTranscriptYet &&
          existing.turnState.messages.length === 0
        ) {
          throw error;
        }
        stored = existing;
      }
    }
    // Re-check AFTER the await: a delete completing while the history
    // request was in flight must not be answered with the pre-delete
    // snapshot captured above (the fallback `existing`).
    if (
      scopeId !== this.ensureScope() ||
      this.deletedConversationIds.has(conversationId)
    ) {
      throw new AssistantConversationNotFoundError();
    }
    this.activateConversation(conversationId, stored);
    return this.historyFromStored(stored);
  }

  private activateConversation(
    conversationId: string,
    stored: StoredConversation,
  ): void {
    if (
      conversationId.startsWith(WORKFLOW_CONVERSATION_PREFIX) &&
      this.activeConversationId !== conversationId
    ) {
      stored.sessionId = crypto.randomUUID();
    }
    this.activeConversationId = conversationId;
  }

  private historyFromStored(
    stored: StoredConversation,
    turnInFlight = false,
  ): ConversationHistory {
    const stalled = stored.projectionStalledAt !== undefined;
    const turnActive =
      turnInFlight || isTurnActive(stored.turnState.activeTurn?.status);
    return {
      conversation: stored.conversation,
      messages: stored.turnState.messages,
      has_more: false,
      ...(stalled
        ? { projectionStalled: true }
        : !turnActive && (stored.identityPending || stored.projectionPending)
          ? { awaitingProjection: true }
          : {}),
    };
  }

  private syntheticPendingConversation(
    conversationId: string,
    facts: Pick<
      StoredConversation,
      "identityPending" | "projectionPending" | "stateVersion"
    >,
  ): StoredConversation {
    const nowIso = new Date(this.now()).toISOString();
    return {
      conversation: {
        id: conversationId,
        title: "Conversation",
        created_at: nowIso,
        last_message_at: nowIso,
      },
      turnState: EMPTY_TURN_STATE,
      actionRequestFingerprints: new Map(),
      activityMessageTurnIds: new Map(),
      ...facts,
    };
  }

  private async fetchRawIndexMembership(
    conversationId: string,
    signal?: AbortSignal,
  ): Promise<boolean | "unavailable"> {
    const scopeId = this.ownerScopeId;
    try {
      const response = await assistantApi.get<AevatarConversationListResponse>(
        `${ASSISTANT_PREFIX}/conversations`,
        signal,
      );
      if (scopeId !== this.ownerScopeId) return "unavailable";
      const entries = response.conversations ?? [];
      const present = entries.some(
        (entry) => entry?.id?.trim() === conversationId,
      );
      const deletionIntentIds = new Set(
        listDeletionIntents().flatMap((intent) =>
          intent.conversationId ? [intent.conversationId] : [],
        ),
      );
      for (const entry of entries) {
        const id = entry?.id?.trim();
        if (id) this.mergeIndexEntry(id, entry, deletionIntentIds);
      }
      this.listFetchedAt = this.now();
      return present;
    } catch (error) {
      if (signal?.aborted) throw error;
      return "unavailable";
    }
  }

  private tombstoneConversation(conversationId: string): void {
    const canonicalId = this.canonicalConversationId(conversationId);
    this.conversations.delete(conversationId);
    this.conversations.delete(canonicalId);
    this.deletedConversationIds.add(conversationId);
    this.deletedConversationIds.add(canonicalId);
    const entry = this.reconcileEntries.get(canonicalId);
    if (entry?.timer !== undefined) {
      clearTimeout(entry.timer);
      entry.timer = undefined;
      entry.nextAttemptAt = undefined;
    }
    entry?.controller?.abort();
    if (entry && !entry.running && entry.waiters > 0) {
      this.resumeReconcileEntry(entry);
    }
  }

  reconcileProjection(
    requestedId: string,
  ): Promise<ProjectionReconcileOutcome> {
    const scopeId = this.ensureScope();
    const placeholderReceipt = requestedId.startsWith(
      PENDING_WORKFLOW_CONVERSATION_PREFIX,
    )
      ? findReceiptByPlaceholder(requestedId)
      : undefined;
    if (placeholderReceipt?.conversationId) {
      this.conversationAliases.set(
        requestedId,
        placeholderReceipt.conversationId,
      );
    }
    let conversationId = this.canonicalConversationId(requestedId);
    let stored = this.conversations.get(conversationId);
    if (!stored && placeholderReceipt && !placeholderReceipt.conversationId) {
      stored = this.syntheticPendingConversation(requestedId, {
        identityPending: true,
        stateVersion: placeholderReceipt.stateVersion,
      });
      this.conversations.set(requestedId, stored);
      conversationId = requestedId;
    }
    if (!stored) {
      const receipt = findReceiptByConversation(conversationId);
      if (receipt) {
        stored = this.syntheticPendingConversation(conversationId, {
          projectionPending: true,
          stateVersion: receipt.stateVersion,
        });
        this.conversations.set(conversationId, stored);
      }
    }

    const existing = this.reconcileEntries.get(conversationId);
    if (existing) {
      existing.waiters += 1;
      this.resumeReconcileEntry(existing);
      return existing.promise;
    }

    const wasStalled = stored?.projectionStalledAt !== undefined;
    if (wasStalled && stored) {
      stored.projectionStalledAt = undefined;
      if (!stored.identityPending) stored.projectionPending = true;
    }
    const commandId =
      placeholderReceipt?.commandId ??
      findReceiptByConversation(conversationId)?.commandId;
    const policy = stored?.identityPending
      ? CREATE_RECOVERY_BACKOFF_POLICY
      : PROJECTION_BACKOFF_POLICY;
    const createdAt = this.now();
    const coldObservationIsCurrent =
      stored?.lastWireObservationAt !== undefined &&
      createdAt - stored.lastWireObservationAt >= 0 &&
      createdAt - stored.lastWireObservationAt <=
        PROJECTION_BACKOFF_POLICY.floorMs;
    const origin: ReconcileOrigin = stored?.identityPending
      ? "identity_recovery"
      : wasStalled
        ? "explicit_retry"
        : coldObservationIsCurrent
          ? "cold_observed"
          : "post_terminal";
    if (origin === "cold_observed" && stored) {
      stored.lastWireObservationAt = undefined;
    }
    const startsImmediately =
      origin === "identity_recovery" || origin === "explicit_retry";
    const initialAttempt = origin === "cold_observed" ? 1 : 0;
    let settle!: (outcome: ProjectionReconcileOutcome) => void;
    const promise = new Promise<ProjectionReconcileOutcome>((resolve) => {
      settle = resolve;
    });
    const entry: ReconcileEntry = {
      promise,
      settle,
      scopeId,
      placeholderId: requestedId.startsWith(
        PENDING_WORKFLOW_CONVERSATION_PREFIX,
      )
        ? requestedId
        : placeholderReceipt?.placeholderId,
      conversationId,
      commandId,
      origin,
      attempt: initialAttempt,
      startedAt: createdAt,
      deadlineAt: createdAt + policy.deadlineMs,
      ...(startsImmediately
        ? {}
        : {
            nextAttemptAt:
              createdAt +
              nextBackoffDelay(
                policy,
                origin === "cold_observed" ? 0 : initialAttempt,
                this.random,
              ),
          }),
      waiters: 1,
      running: false,
      finalObservationDue: false,
    };
    this.reconcileEntries.set(conversationId, entry);
    this.resumeReconcileEntry(entry);
    return promise;
  }

  releaseProjectionWaiter(requestedId: string): void {
    this.ensureScope();
    const conversationId = this.canonicalConversationId(requestedId);
    const entry =
      this.reconcileEntries.get(conversationId) ??
      this.reconcileEntries.get(requestedId);
    if (!entry) return;
    entry.waiters = Math.max(0, entry.waiters - 1);
    if (entry.waiters > 0) return;
    if (entry.timer !== undefined) {
      clearTimeout(entry.timer);
      entry.timer = undefined;
    }
    if (entry.controller) {
      entry.controller.abort();
    } else {
      entry.running = false;
    }
  }

  private resumeReconcileEntry(entry: ReconcileEntry): void {
    if (
      entry.running ||
      entry.timer !== undefined ||
      entry.waiters === 0 ||
      entry.scopeId !== this.ownerScopeId
    ) {
      return;
    }
    const remainingDelay = (entry.nextAttemptAt ?? this.now()) - this.now();
    if (remainingDelay > 0) {
      entry.timer = setTimeout(() => {
        entry.timer = undefined;
        entry.finalObservationDue = this.now() >= entry.deadlineAt;
        entry.nextAttemptAt = undefined;
        this.resumeReconcileEntry(entry);
      }, remainingDelay);
      return;
    }
    entry.nextAttemptAt = undefined;
    entry.finalObservationDue ||= this.now() >= entry.deadlineAt;
    entry.running = true;
    void this.runReconcileObservation(entry).catch(() => {
      if (entry.scopeId !== this.ownerScopeId || entry.waiters === 0) return;
      this.scheduleReconcileEntry(entry);
    });
  }

  private async runReconcileObservation(entry: ReconcileEntry): Promise<void> {
    if (entry.scopeId !== this.ownerScopeId || entry.waiters === 0) {
      entry.running = false;
      return;
    }
    if (
      this.deletedConversationIds.has(entry.conversationId) ||
      this.deletingConversations.has(entry.conversationId)
    ) {
      this.settleReconcileEntry(entry, "absent");
      return;
    }
    const activeStored = this.conversations.get(entry.conversationId);
    const turnInFlight =
      this.running.has(entry.conversationId) ||
      (entry.placeholderId !== undefined &&
        this.running.has(entry.placeholderId));
    if (
      activeStored &&
      (turnInFlight || isTurnActive(activeStored.turnState.activeTurn?.status))
    ) {
      const policy = activeStored.identityPending
        ? CREATE_RECOVERY_BACKOFF_POLICY
        : PROJECTION_BACKOFF_POLICY;
      entry.deadlineAt = this.now() + policy.deadlineMs;
      entry.running = false;
      this.scheduleReconcileEntry(entry);
      return;
    }

    const controller = this.scopeController();
    entry.controller = controller;
    let pausedByAbort = false;
    let transcriptWasMissing = false;
    let observedMembership: boolean | "unavailable" | undefined;
    let rescheduleAfterTurn = false;
    try {
      const stored = this.conversations.get(entry.conversationId);
      if (stored?.identityPending) {
        if (!entry.commandId) {
          this.settleReconcileEntry(entry, "timed_out");
          return;
        }
        try {
          const response =
            await assistantApi.get<AevatarCreateRecoveryResponse>(
              `${ASSISTANT_PREFIX}/conversations/create-recovery/${encodeURIComponent(entry.commandId)}`,
              controller.signal,
            );
          if (entry.scopeId !== this.ownerScopeId) return;
          const recovered = this.decodeCreateRecovery(response);
          this.adoptRecoveredReceipt(entry, stored, recovered);
        } catch (error) {
          if (controller.signal.aborted) {
            pausedByAbort = true;
            return;
          }
          if (!(error instanceof ApiError && error.status === 404)) throw error;
          transcriptWasMissing = true;
        }
      } else {
        try {
          const body = await assistantApi.get<AevatarHistoryResponse>(
            `${ASSISTANT_PREFIX}/conversations/${entry.conversationId}`,
            controller.signal,
          );
          if (entry.scopeId !== this.ownerScopeId) return;
          const postFetchStored = this.conversations.get(entry.conversationId);
          const postFetchTurnInFlight =
            this.running.has(entry.conversationId) ||
            (entry.placeholderId !== undefined &&
              this.running.has(entry.placeholderId));
          if (
            postFetchStored &&
            (postFetchTurnInFlight ||
              isTurnActive(postFetchStored.turnState.activeTurn?.status))
          ) {
            const policy = postFetchStored.identityPending
              ? CREATE_RECOVERY_BACKOFF_POLICY
              : PROJECTION_BACKOFF_POLICY;
            entry.deadlineAt = this.now() + policy.deadlineMs;
            rescheduleAfterTurn = true;
            return;
          }
          const projected = this.applyHistoryResponse(
            entry.conversationId,
            body,
          );
          if (!projected.identityPending && !projected.projectionPending) {
            this.settleReconcileEntry(entry, "materialized");
            return;
          }
        } catch (error) {
          if (controller.signal.aborted) {
            pausedByAbort = true;
            return;
          }
          if (!(error instanceof ApiError && error.status === 404)) throw error;
          transcriptWasMissing = true;
        }
      }

      entry.attempt += 1;
      const deadlineReached =
        entry.finalObservationDue || this.now() >= entry.deadlineAt;
      entry.finalObservationDue = false;
      if (
        !this.conversations.get(entry.conversationId)?.identityPending &&
        transcriptWasMissing &&
        (entry.attempt % 2 === 0 || deadlineReached)
      ) {
        const membership = await this.fetchRawIndexMembership(
          entry.conversationId,
          controller.signal,
        );
        observedMembership = membership;
        if (entry.scopeId !== this.ownerScopeId) return;
        if (membership === false) {
          if (deadlineReached) {
            this.tombstoneConversation(entry.conversationId);
            this.settleReconcileEntry(entry, "absent");
            return;
          }
          if (
            entry.firstAbsentAt !== undefined &&
            this.now() - entry.firstAbsentAt >= 10_000
          ) {
            this.tombstoneConversation(entry.conversationId);
            this.settleReconcileEntry(entry, "absent");
            return;
          }
          entry.firstAbsentAt ??= this.now();
        } else if (membership === true) {
          entry.firstAbsentAt = undefined;
        }
      }

      if (deadlineReached) {
        const identityPending =
          this.conversations.get(entry.conversationId)?.identityPending ===
          true;
        const membership = identityPending
          ? "unavailable"
          : (observedMembership ??
            (await this.fetchRawIndexMembership(
              entry.conversationId,
              controller.signal,
            )));
        if (membership === false) {
          this.tombstoneConversation(entry.conversationId);
          this.settleReconcileEntry(entry, "absent");
        } else {
          this.settleReconcileEntry(entry, "timed_out");
        }
        return;
      }
    } finally {
      if (entry.controller === controller) entry.controller = undefined;
      this.releaseScopeController(controller);
      entry.running = false;
      if (
        pausedByAbort &&
        entry.waiters > 0 &&
        entry.scopeId === this.ownerScopeId &&
        this.reconcileEntries.has(entry.conversationId)
      ) {
        this.scheduleReconcileEntry(entry);
      } else if (rescheduleAfterTurn) {
        this.scheduleReconcileEntry(entry);
      }
    }
    this.scheduleReconcileEntry(entry);
  }

  private scheduleReconcileEntry(entry: ReconcileEntry): void {
    if (
      entry.scopeId !== this.ownerScopeId ||
      entry.waiters === 0 ||
      !this.reconcileEntries.has(entry.conversationId)
    ) {
      return;
    }
    if (entry.nextAttemptAt === undefined) {
      const identityPending =
        this.conversations.get(entry.conversationId)?.identityPending === true;
      const policy = identityPending
        ? CREATE_RECOVERY_BACKOFF_POLICY
        : PROJECTION_BACKOFF_POLICY;
      entry.nextAttemptAt =
        this.now() +
        nextBackoffDelay(policy, Math.max(0, entry.attempt - 1), this.random);
    }
    this.resumeReconcileEntry(entry);
  }

  private adoptRecoveredReceipt(
    entry: ReconcileEntry,
    stored: StoredConversation,
    recovery: ReturnType<AevatarAssistantTransport["decodeCreateRecovery"]>,
  ): void {
    const placeholderId = entry.placeholderId ?? entry.conversationId;
    stored.conversation = {
      ...stored.conversation,
      id: recovery.conversationId,
    };
    stored.identityPending = false;
    stored.projectionPending = true;
    stored.requiredTurnId = recovery.turnId;
    stored.stateVersion = Math.max(
      stored.stateVersion ?? 0,
      recovery.stateVersion,
    );
    stored.createRequest = undefined;
    this.conversations.set(recovery.conversationId, stored);
    this.conversations.set(placeholderId, stored);
    this.conversationAliases.set(placeholderId, recovery.conversationId);
    if (this.activeConversationId === placeholderId) {
      this.activeConversationId = recovery.conversationId;
    }
    adoptReceiptIdentity(
      entry.commandId ?? "",
      recovery.conversationId,
      recovery.stateVersion,
      this.now(),
    );
    this.reconcileEntries.delete(entry.conversationId);
    entry.conversationId = recovery.conversationId;
    this.reconcileEntries.set(entry.conversationId, entry);
  }

  private settleReconcileEntry(
    entry: ReconcileEntry,
    status: ProjectionReconcileOutcome["status"],
  ): void {
    if (entry.timer !== undefined) clearTimeout(entry.timer);
    entry.controller?.abort();
    const stored = this.conversations.get(entry.conversationId);
    if (status === "timed_out" && stored) {
      stored.projectionStalledAt = this.now();
    }
    this.reconcileEntries.delete(entry.conversationId);
    if (entry.placeholderId) this.reconcileEntries.delete(entry.placeholderId);
    entry.settle({ status, conversationId: entry.conversationId });
  }

  private appendOptimisticUserMessage(
    conversationId: string,
    run: RunningTurn,
    content: string,
  ): void {
    if (run.optimisticMessageAppended) return;
    const stored = this.conversations.get(conversationId);
    if (!stored) return;

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
              text: content,
            },
          ],
          created_at: createdAt,
        },
      ],
      lastCursor: 0,
    };
    stored.conversation = {
      ...stored.conversation,
      title: firstMessage ? content.slice(0, 40) : stored.conversation.title,
      last_message_at: createdAt,
    };
    run.optimisticMessageAppended = true;
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    this.ensureScope();
    const requestedId = conversationId;
    conversationId = this.canonicalConversationId(conversationId);
    const stored = this.conversations.get(conversationId);
    if (!stored) {
      throw new AssistantConversationNotFoundError();
    }
    if (this.deletingConversations.has(conversationId)) {
      throw new Error("This conversation is being deleted.");
    }
    if (
      this.running.has(conversationId) ||
      this.running.has(requestedId) ||
      isTurnActive(stored.turnState.activeTurn?.status)
    ) {
      throw new AssistantTurnActiveError();
    }
    const normalized = content.trim();
    if (!normalized || normalized.length > MAX_MESSAGE_CHARS) {
      throw new Error("Message must contain between 1 and 32768 characters.");
    }

    const protocol = isWorkflowConversationId(conversationId)
      ? "workflow"
      : "actor";
    let clientRequestId: string | undefined;
    if (
      protocol === "workflow" &&
      !stored.conversation.id.startsWith(WORKFLOW_CONVERSATION_PREFIX)
    ) {
      clientRequestId =
        stored.createRequest?.prompt === normalized
          ? stored.createRequest.commandId
          : crypto.randomUUID();
      stored.createRequest = { prompt: normalized, commandId: clientRequestId };
      stored.identityPending = true;
      stored.projectionStalledAt = undefined;
      recordCreateReceipt(clientRequestId, requestedId, this.now());
    }
    const run = this.newRun(onEvent, null, protocol, clientRequestId);
    run.assistantMessageIdsAtDispatch = new Set(
      stored.turnState.messages
        .filter((message) => message.role === "assistant")
        .map((message) => message.id),
    );
    this.running.set(conversationId, run);
    const needsWorkflowPreflight =
      protocol === "workflow" &&
      stored.conversation.id.startsWith(WORKFLOW_CONVERSATION_PREFIX) &&
      positiveStateVersion(stored.stateVersion) === undefined;
    if (!needsWorkflowPreflight) {
      this.appendOptimisticUserMessage(conversationId, run, normalized);
    }
    void this.streamTurn(conversationId, run, normalized);
    return {
      get turnId() {
        return run.turnId;
      },
      cancel: () => {
        this.cancelTurn(conversationId, run);
      },
    };
  }

  /**
   * Cancel whatever turn is live for the conversation — including an
   * approval-continuation reservation whose handle the caller never saw
   * (the handle only returns after the approve response headers arrive, so
   * Stop needs a transport-level lookup to abort a hung request).
   */
  cancelActiveTurn(conversationId: string): void {
    this.ensureScope();
    // A create-and-first-turn run is keyed under the placeholder id the send
    // used. Resolve forward and reverse so either the placeholder or the
    // canonical address can stop it, then cancel under the run's own key.
    const canonicalId = this.canonicalConversationId(conversationId);
    const addresses = new Set([conversationId, canonicalId]);
    for (const [placeholderId, targetId] of this.conversationAliases) {
      if (targetId === canonicalId) addresses.add(placeholderId);
    }
    for (const key of addresses) {
      const run = this.running.get(key);
      if (run) {
        this.cancelTurn(key, run);
        return;
      }
    }
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
    this.ensureScope();
    conversationId = this.canonicalConversationId(conversationId);
    if (this.deletingConversations.has(conversationId)) {
      throw new Error("This conversation is being deleted.");
    }
    // `:approve` addresses a nyxid-chat conversation ACTOR. A workflow run
    // resumes through `runs/{runId}:resume`, which this mount does not
    // proxy — posting the actor route with a `chatc-…` id would 404 with a
    // misleading message. Fail honestly instead: the gate is real, we just
    // cannot decide it from here yet.
    if (isWorkflowConversationId(conversationId)) {
      throw new Error(
        "Approvals cannot be decided from this chat yet. Approve the request in NyxID (Approvals) and send your message again.",
      );
    }
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

    // Reserve the conversation BEFORE the network call: the whole approve
    // exchange must read as one active turn, or the idle gap while awaiting
    // response headers lets a concurrent send/approve/delete slip past the
    // active-turn guards and interleave two streams into one reducer. The
    // reservation's controller doubles as the fetch signal so Stop can
    // abort an approve request hung before headers.
    const run = this.newRun(
      onEvent ?? noopEvent,
      active?.turnId ?? stored.turnState.activeTurn?.turnId ?? null,
    );
    // Continuation cursors continue past the previous turn's: the reducer
    // and any still-subscribed pump dedup by strictly-increasing cursor.
    // (Read lastCursor only after pauseForApproval settled the prior turn.)
    run.cursor = stored.turnState.lastCursor;
    this.running.set(conversationId, run);

    // Reservation first, THEN the fence: the approve must not overtake a
    // prior turn's still-pending stop upstream, and the reservation keeps
    // concurrent sends out while this waits. A cancel landing during the
    // wait settles the run before anything was dispatched — bail with no
    // continuation rather than posting a decision for a cancelled flow.
    await this.awaitPendingStop(conversationId);
    if (run.finished || run.controller.signal.aborted) {
      return null;
    }

    const stream = this.startChatStream(
      conversationId,
      run,
      TYPED_CHAT_URL,
      JSON.stringify({
        type: "approval.resolve",
        conversationId,
        clientRequestId: crypto.randomUUID(),
        requestId: card.approval_request_id,
        approved,
      }),
    );
    const response = await stream.headers;
    if (response.kind === "cancelled") {
      // cancelTurn already emitted the terminal events when the user stopped
      // this request before response headers arrived.
      this.finishTurn(conversationId, run, "cancelled", null);
      throw new AssistantTurnCancelledError();
    }
    if (response.kind === "network_error") {
      const aborted = run.controller.signal.aborted;
      // cancelTurn may already have settled the run; finishTurn is a no-op
      // then. The card was never flipped, so the decision stays retryable.
      // Pre-stream failures settle the turn with a NULL error: the thrown
      // rejection is what surfaces (the mutation's onError toast) — a turn
      // error here would double-toast the same failure.
      this.finishTurn(
        conversationId,
        run,
        aborted ? "cancelled" : "failed",
        null,
      );
      throw aborted
        ? new AssistantTurnCancelledError()
        : new Error(response.message);
    }
    if (response.kind === "http_error") {
      const failure = streamStartError(response.status, response.body);
      // Null turn error for the same single-toast reason as above.
      this.finishTurn(conversationId, run, "failed", null);
      throw new Error(failure.message);
    }

    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.updated",
      block_id: blockId,
      patch: {
        decision: approved ? "approved" : "denied",
        decision_channel: "web",
      },
    });
    // The prior turn's ledger parked with a step waiting on THIS approval;
    // settle that step so the transient activity line doesn't show a stale
    // approval clock (approved → the step proceeds; denied → skipped).
    // Correlated by approval_request_id: deciding one card must not settle
    // steps gated on a different pending approval, and a ledger with other
    // approvals still waiting stays parked.
    const parkedLedger = [...stored.turnState.messages]
      .flatMap((message) => message.blocks)
      .reverse()
      .find(
        (candidate): candidate is RunContentBlock =>
          candidate.type === "run" &&
          candidate.state === "awaiting_approval" &&
          candidate.steps.some(
            (step) =>
              step.status === "waiting" &&
              step.approval_request_id === card.approval_request_id,
          ),
      );
    if (parkedLedger) {
      const steps = parkedLedger.steps.map((step) =>
        step.status === "waiting" &&
        step.approval_request_id === card.approval_request_id
          ? {
              ...step,
              status: approved ? ("done" as const) : ("skipped" as const),
            }
          : step,
      );
      const stillWaiting = steps.some((step) => step.status === "waiting");
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.updated",
        block_id: parkedLedger.block_id,
        patch: {
          state: stillWaiting
            ? "awaiting_approval"
            : approved
              ? "completed"
              : "cancelled",
          steps,
          steps_complete: steps.filter((step) => step.status === "done").length,
        },
      });
    }

    if (response.contentType.includes("text/event-stream")) {
      void this.consumeApprovalContinuation(conversationId, run, stream);
    } else {
      // Older backend acknowledging with JSON: nothing further will stream,
      // and there is no live continuation for the caller to hold a handle
      // to — returning one would let a stale entry linger in the caller's
      // handle registry after this turn already completed.
      stream.cancel();
      this.finishTurn(conversationId, run, "completed", null);
      return null;
    }
    return {
      get turnId() {
        return run.turnId;
      },
      cancel: () => {
        this.cancelTurn(conversationId, run);
      },
    };
  }

  setActionCardInProgress(
    conversationId: string,
    blockId: string,
    inProgress: boolean,
    onEvent: (event: TurnEvent) => void = noopEvent,
  ): void {
    this.ensureScope();
    if (
      this.deletingConversations.has(
        this.canonicalConversationId(conversationId),
      )
    ) {
      throw new Error("This conversation is being deleted.");
    }
    const card = this.findActionCard(conversationId, blockId);
    if (!card) throw new Error("Action request was not found.");
    if (
      card.status === "blocked" ||
      card.status === "completed" ||
      card.status === "conflicted" ||
      card.status === "declined" ||
      card.status === "failed" ||
      card.status === "unsupported"
    ) {
      return;
    }
    this.emitLocalBlockPatch(
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
    onEvent: (event: TurnEvent) => void = noopEvent,
  ): void {
    this.ensureScope();
    if (!this.conversations.has(conversationId)) {
      throw new Error("Conversation was not found.");
    }
    if (
      this.deletingConversations.has(
        this.canonicalConversationId(conversationId),
      )
    ) {
      throw new Error("This conversation is being deleted.");
    }
    const card = this.findActionCard(conversationId, blockId);
    if (!card) throw new Error("Action request was not found.");
    if (
      card.status === "completed" ||
      card.status === "conflicted" ||
      card.status === "declined" ||
      card.status === "failed"
    ) {
      return;
    }
    this.emitLocalBlockPatch(
      conversationId,
      blockId,
      {
        status: "blocked",
        outcome_note: note,
      },
      onEvent,
    );
  }

  continueActions(
    conversationId: string,
    originTurnId: string,
    reports: readonly ActionReport[],
    onEvent: (event: TurnEvent) => void = noopEvent,
  ): TurnHandle | null {
    this.ensureScope();
    if (!this.conversations.has(conversationId)) {
      throw new AssistantConversationNotFoundError();
    }
    if (
      this.deletingConversations.has(
        this.canonicalConversationId(conversationId),
      )
    ) {
      throw new Error("This conversation is being deleted.");
    }
    const validatedReports = reports.map((report) =>
      actionReportSchema.parse(report),
    );
    const reportActionLookup = new Map<string, string>();
    for (const report of validatedReports) {
      const card = this.findActionCardByRequestId(
        conversationId,
        report.actionRequestId,
      );
      if (card?.action) {
        reportActionLookup.set(report.actionRequestId, card.action);
      }
      const refusedByCardState =
        card?.status === "conflicted" ||
        (card?.status === "blocked" && report.disposition === "completed");
      if (refusedByCardState) {
        if (card && report.disposition === "completed" && report.resource) {
          this.emitLocalBlockPatch(
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
        throw new AssistantProtocolError(
          "This action request can no longer be continued from the current card state.",
        );
      }
    }
    // Validate origin matching, non-empty actions and duplicate ids before
    // changing any card. The real client id is allocated only when drained.
    buildActionContinueBody(
      this.canonicalConversationId(conversationId),
      "validation",
      originTurnId,
      validatedReports,
      reportActionLookup,
    );

    const stored = this.conversations.get(conversationId);
    const actorIds = new Set(
      validatedReports
        .map(
          (report) =>
            this.findActionCardByRequestId(
              conversationId,
              report.actionRequestId,
            )?.actor_id,
        )
        .filter((actorId): actorId is string => Boolean(actorId)),
    );
    if (actorIds.size > 1) {
      throw new AssistantProtocolError(
        "Action reports from different conversation actors cannot share a batch.",
      );
    }
    const actorId =
      actorIds.values().next().value ??
      (stored && !isWorkflowConversationId(stored.conversation.id)
        ? stored.conversation.id
        : null);

    const batches = this.pendingActionBatches.get(conversationId) ?? [];
    let batch = [...batches]
      .reverse()
      .find(
        (candidate) =>
          candidate.originTurnId === originTurnId &&
          candidate.actorId === actorId &&
          candidate.clientRequestId === null &&
          !candidate.inFlight &&
          !candidate.blocked,
      );
    if (!batch) {
      batch = {
        id: newId("action-batch"),
        originTurnId,
        actorId,
        reports: new Map(),
        onEvent,
        clientRequestId: null,
        inFlight: false,
        blocked: false,
      };
      batches.push(batch);
      this.pendingActionBatches.set(conversationId, batches);
    } else {
      batch.onEvent = onEvent;
    }

    for (const report of validatedReports) {
      const alreadyQueued = batches.some((candidate) =>
        candidate.reports.has(report.actionRequestId),
      );
      if (alreadyQueued) continue;
      batch.reports.set(report.actionRequestId, report);
      const card = this.findActionCardByRequestId(
        conversationId,
        report.actionRequestId,
      );
      if (card) {
        this.emitLocalBlockPatch(
          conversationId,
          card.block_id,
          {
            status: this.actionStatusForDisposition(report.disposition),
            outcome_note: this.actionOutcomeNote(report.disposition),
          },
          onEvent,
        );
      }
    }

    return this.drainPendingActions(conversationId);
  }

  wakeActions(
    conversationId: string,
    originTurnId: string,
    onEvent: (event: TurnEvent) => void = noopEvent,
  ): TurnHandle {
    this.ensureScope();
    const requestedId = conversationId;
    const actorId = this.canonicalConversationId(conversationId);
    const stored =
      this.conversations.get(requestedId) ?? this.conversations.get(actorId);
    if (!stored) throw new AssistantConversationNotFoundError();
    if (this.deletingConversations.has(actorId)) {
      throw new Error("This conversation is being deleted.");
    }
    if (!TYPED_SERVER_CONVERSATION_ID_PATTERN.test(actorId)) {
      throw new AssistantProtocolError(
        "Action wakes require a typed assistant conversation.",
      );
    }
    if (
      this.running.has(requestedId) ||
      this.running.has(actorId) ||
      isTurnActive(stored.turnState.activeTurn?.status)
    ) {
      throw new AssistantTurnActiveError();
    }

    const body = buildActionWakeBody(
      actorId,
      crypto.randomUUID(),
      originTurnId,
    );
    const run = this.newRun(onEvent, null, "actor");
    run.cursor = stored.turnState.lastCursor;
    this.running.set(requestedId, run);
    void this.streamActionContinuation(requestedId, run, body);
    return {
      get turnId() {
        return run.turnId;
      },
      cancel: () => this.cancelTurn(requestedId, run),
    };
  }

  private findActionCard(
    conversationId: string,
    blockId: string,
  ): ActionCardContentBlock | undefined {
    return this.conversations
      .get(conversationId)
      ?.turnState.messages.flatMap((message) => message.blocks)
      .find(
        (block): block is ActionCardContentBlock =>
          block.type === "action_card" && block.block_id === blockId,
      );
  }

  private findActionCardByRequestId(
    conversationId: string,
    actionRequestId: string,
  ): ActionCardContentBlock | undefined {
    return this.conversations
      .get(conversationId)
      ?.turnState.messages.flatMap((message) => message.blocks)
      .find(
        (block): block is ActionCardContentBlock =>
          block.type === "action_card" &&
          block.action_request_id === actionRequestId,
      );
  }

  private emitLocalBlockPatch(
    conversationId: string,
    blockId: string,
    patch: Partial<ActionCardContentBlock>,
    onEvent: (event: TurnEvent) => void,
  ): void {
    const stored = this.conversations.get(conversationId);
    if (!stored) throw new AssistantConversationNotFoundError();
    const activeRun = this.running.get(conversationId);
    if (activeRun) {
      this.emit(conversationId, activeRun, {
        cursor: this.nextCursor(activeRun),
        event: "block.updated",
        block_id: blockId,
        patch,
      });
      return;
    }
    const event: TurnEvent = {
      cursor: stored.turnState.lastCursor + 1,
      event: "block.updated",
      block_id: blockId,
      patch,
    };
    stored.turnState = applyTurnEvent(stored.turnState, event);
    stored.conversation = {
      ...stored.conversation,
      last_message_at: new Date().toISOString(),
    };
    onEvent(event);
  }

  private actionStatusForDisposition(
    disposition: ActionReport["disposition"],
  ): ActionCardStatus {
    if (disposition === "completed") return "completed";
    if (disposition === "declined") return "declined";
    return "failed";
  }

  private actionOutcomeNote(disposition: ActionReport["disposition"]): string {
    if (disposition === "completed") {
      return "Reporting the new service reference to the assistant.";
    }
    if (disposition === "declined") {
      return "You declined this request. Sending the decision to the assistant; no credential was shared.";
    }
    return "The connection could not be completed. Sending the failure to the assistant.";
  }

  private settledActionOutcomeNote(
    disposition: ActionReport["disposition"],
    delivered: boolean,
  ): string {
    if (disposition === "completed") {
      return delivered
        ? "Reported — awaiting assistant verification."
        : "The new service reference has not reached the assistant; delivery will retry after the next turn.";
    }
    if (disposition === "declined") {
      return delivered
        ? "You declined this request. The assistant received the decision; no credential was shared."
        : "You declined this request. The decision has not reached the assistant; delivery will retry after the next turn.";
    }
    return delivered
      ? "The assistant received the connection failure. Ask it to request the service again."
      : "The connection failure has not reached the assistant; delivery will retry after the next turn.";
  }

  private updateActionBatchOutcomeNotes(
    conversationId: string,
    batch: PendingActionBatch,
    delivered: boolean,
  ): void {
    for (const report of batch.reports.values()) {
      const card = this.findActionCardByRequestId(
        conversationId,
        report.actionRequestId,
      );
      if (!card) continue;
      this.emitLocalBlockPatch(
        conversationId,
        card.block_id,
        {
          outcome_note: this.settledActionOutcomeNote(
            report.disposition,
            delivered,
          ),
        },
        batch.onEvent,
      );
    }
  }

  private drainPendingActions(conversationId: string): TurnHandle | null {
    const stored = this.conversations.get(conversationId);
    if (
      !stored ||
      this.running.has(conversationId) ||
      isTurnActive(stored.turnState.activeTurn?.status) ||
      this.actionDrainBlocked.has(conversationId)
    ) {
      return null;
    }
    const batch = this.pendingActionBatches
      .get(conversationId)
      ?.find(
        (candidate) =>
          candidate.reports.size > 0 &&
          !candidate.inFlight &&
          !candidate.blocked,
      );
    if (!batch) return null;

    // `action.continue` belongs to the nyxid-chat actor protocol even when the
    // visible conversation runs on the studio workflow surface. The action
    // frame carries its owning ConversationActorId; never substitute the
    // workflow `chatc-*` id or send the control body to `/workflow-chat`.
    if (!batch.actorId) {
      batch.blocked = true;
      this.actionDrainBlocked.add(conversationId);
      this.updateActionBatchOutcomeNotes(conversationId, batch, false);
      return null;
    }

    batch.clientRequestId ??= crypto.randomUUID();
    const reportActionLookup = new Map(
      [...batch.reports.keys()].flatMap((actionRequestId) => {
        const action = this.findActionCardByRequestId(
          conversationId,
          actionRequestId,
        )?.action;
        return action ? [[actionRequestId, action] as const] : [];
      }),
    );
    const body = buildActionContinueBody(
      batch.actorId,
      batch.clientRequestId,
      batch.originTurnId,
      [...batch.reports.values()],
      reportActionLookup,
    );
    const run = this.newRun(batch.onEvent, null, "actor");
    run.cursor = stored.turnState.lastCursor;
    run.actionContinuation = {
      batchId: batch.id,
      retryQueued: false,
      settled: false,
    };
    batch.inFlight = true;
    this.running.set(conversationId, run);
    void this.streamActionContinuation(conversationId, run, body);
    return {
      get turnId() {
        return run.turnId;
      },
      cancel: () => this.cancelTurn(conversationId, run),
    };
  }

  private findActionBatch(
    conversationId: string,
    batchId: string,
  ): PendingActionBatch | undefined {
    return this.pendingActionBatches
      .get(conversationId)
      ?.find((batch) => batch.id === batchId);
  }

  /** Drop the batch: the server admitted the reports and started the turn. */
  private acceptActionBatch(conversationId: string, run: RunningTurn): void {
    const state = run.actionContinuation;
    if (!state || state.settled) return;
    state.settled = true;
    const batches = this.pendingActionBatches.get(conversationId);
    if (!batches) return;
    const index = batches.findIndex((batch) => batch.id === state.batchId);
    if (index >= 0) {
      const batch = batches[index];
      if (batch)
        this.updateActionBatchOutcomeNotes(conversationId, batch, true);
      batches.splice(index, 1);
    }
    if (batches.length === 0) this.pendingActionBatches.delete(conversationId);
  }

  private keepActionBatchQueued(
    conversationId: string,
    run: RunningTurn,
  ): void {
    const state = run.actionContinuation;
    if (!state || state.settled) return;
    state.settled = true;
    const batch = this.findActionBatch(conversationId, state.batchId);
    if (batch) {
      batch.inFlight = false;
      batch.blocked = true;
      this.updateActionBatchOutcomeNotes(conversationId, batch, false);
    }
    state.retryQueued = true;
    this.actionDrainBlocked.add(conversationId);
  }

  private unblockActionBatches(conversationId: string): void {
    this.actionDrainBlocked.delete(conversationId);
    for (const batch of this.pendingActionBatches.get(conversationId) ?? []) {
      batch.blocked = false;
    }
  }

  /**
   * Fold one Chat History index row into the local mirror. A conversation
   * with a live turn keeps its in-flight mirror (index metadata must not
   * clobber a streaming transcript); everything else adopts the server's
   * title, timestamps, and counts without a per-conversation detail fetch.
   */
  private mergeIndexEntry(
    id: string,
    entry: AevatarHistoryIndexEntry,
    deletionIntentIds: ReadonlySet<string>,
  ): void {
    if (this.deletedConversationIds.has(id)) return;
    if (deletionIntentIds.has(id)) return;
    const existing = this.conversations.get(id);
    if (existing && isTurnActive(existing.turnState.activeTurn?.status)) {
      return;
    }
    const epoch0 = new Date(0).toISOString();
    const createdAt = indexTimestampToIso(
      entry.createdAt,
      existing?.conversation.created_at ?? epoch0,
    );
    const lastMessageAt = indexTimestampToIso(
      entry.updatedAt ?? entry.createdAt,
      existing?.conversation.last_message_at ?? createdAt,
    );
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
      actionRequestFingerprints:
        existing?.actionRequestFingerprints ?? new Map(),
      activityMessageTurnIds: existing?.activityMessageTurnIds ?? new Map(),
      lastLocalTurnCompletedAt: existing?.lastLocalTurnCompletedAt,
      stateVersion: existing?.stateVersion,
      identityPending: existing?.identityPending,
      projectionPending: existing?.projectionPending,
      requiredTurnId: existing?.requiredTurnId,
      requiredAssistantBaselineIds: existing?.requiredAssistantBaselineIds,
      projectionStalledAt: existing?.projectionStalledAt,
      lastWireObservationAt: existing?.lastWireObservationAt,
      sessionId: existing?.sessionId,
      createRequest: existing?.createRequest,
    });
  }

  private async loadHistory(
    conversationId: string,
  ): Promise<StoredConversation> {
    const body = await assistantApi.get<AevatarHistoryResponse>(
      `${ASSISTANT_PREFIX}/conversations/${conversationId}`,
    );
    return this.applyHistoryResponse(conversationId, body);
  }

  private applyHistoryResponse(
    conversationId: string,
    body: AevatarHistoryResponse,
  ): StoredConversation {
    // The conversation may have been deleted while this request was in
    // flight (deleting an active chat races cancel-driven projections);
    // writing the stale result back would resurrect it locally.
    if (this.deletedConversationIds.has(conversationId)) {
      throw new AssistantConversationNotFoundError();
    }
    const existing = this.conversations.get(conversationId);
    if (existing && isTurnActive(existing.turnState.activeTurn?.status)) {
      return existing;
    }
    const entries = readHistoryEntries(body);
    const messages = entries
      .map((entry, index) => historyEntryToMessage(entry, index))
      .filter((message): message is AssistantMessage => message !== null);
    // The watermark is fresher than any stored one even when the transcript
    // below keeps the local mirror (keep-max), so capture it first.
    const freshStateVersion = historyStateVersion(body);
    const preMergeFence = Math.max(
      1,
      positiveStateVersion(existing?.stateVersion) ?? 1,
    );
    if (existing && freshStateVersion !== undefined) {
      existing.stateVersion = Math.max(
        existing.stateVersion ?? 0,
        freshStateVersion,
      );
      advanceReceiptFence(conversationId, freshStateVersion, this.now());
    }
    const localMessages = existing?.turnState.messages ?? [];
    const localStructuredBlocks = structuredBlockCount(localMessages);
    const serverStructuredBlocks = structuredBlockCount(messages);
    const serverLacksLocalStructure =
      localStructuredBlocks > serverStructuredBlocks;
    // Structured activity messages are not part of the text-only History v4
    // transcript. Exclude them from keep-max's length comparison or their
    // permanent retention would also permanently pin optimistic text ids.
    const comparableLocalMessageCount = serverLacksLocalStructure
      ? localMessages.filter((message) => !hasStructuredBlocks(message)).length
      : localMessages.length;
    const withinMaterializationGrace =
      existing?.lastLocalTurnCompletedAt !== undefined &&
      this.now() - existing.lastLocalTurnCompletedAt <=
        HISTORY_MATERIALIZATION_GRACE_MS;
    if (existing && comparableLocalMessageCount > messages.length) {
      const latestTurnId = this.latestAssistantTurnId(existing);
      const canReplaceLongerLocal =
        freshStateVersion !== undefined &&
        freshStateVersion >= preMergeFence &&
        !withinMaterializationGrace &&
        historyIncludesAssistantTurn(entries, latestTurnId);
      if (!canReplaceLongerLocal) {
        this.applyMaterializationObservation(existing, body, entries);
        return existing;
      }
    }
    if (
      existing &&
      serverLacksLocalStructure &&
      comparableLocalMessageCount === messages.length &&
      withinMaterializationGrace
    ) {
      this.applyMaterializationObservation(existing, body, entries);
      return existing;
    }
    const projectedMessages =
      existing && serverLacksLocalStructure
        ? preserveLocalStructuredMessages(
            messages,
            localMessages,
            existing.activityMessageTurnIds,
          )
        : messages;
    const first = projectedMessages[0];
    const last = projectedMessages[projectedMessages.length - 1];
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
        messages: projectedMessages,
        activeTurn: existing?.turnState.activeTurn ?? null,
        lastCursor: existing?.turnState.lastCursor ?? 0,
      },
      actionRequestFingerprints:
        existing?.actionRequestFingerprints ?? new Map(),
      activityMessageTurnIds: existing?.activityMessageTurnIds ?? new Map(),
      lastLocalTurnCompletedAt: existing?.lastLocalTurnCompletedAt,
      stateVersion:
        freshStateVersion === undefined
          ? existing?.stateVersion
          : Math.max(existing?.stateVersion ?? 0, freshStateVersion),
      identityPending: existing?.identityPending,
      projectionPending: existing?.projectionPending,
      requiredTurnId: existing?.requiredTurnId,
      // The baseline fence must survive transcript application: an
      // equal-length fence-current read that carries the newly committed
      // user row but not yet the assistant row would otherwise present no
      // fence at all and settle materialization prematurely — nothing would
      // ever schedule the read that renders the reply.
      requiredAssistantBaselineIds: existing?.requiredAssistantBaselineIds,
      projectionStalledAt: existing?.projectionStalledAt,
      lastWireObservationAt: existing?.lastWireObservationAt,
      sessionId: existing?.sessionId,
      createRequest: existing?.createRequest,
    };
    this.applyMaterializationObservation(stored, body, entries);
    this.conversations.set(conversationId, stored);
    return stored;
  }

  private applyMaterializationObservation(
    stored: StoredConversation,
    body: AevatarHistoryResponse,
    entries: readonly AevatarHistoryEntry[],
  ): void {
    const freshStateVersion = historyStateVersion(body);
    const containsRequiredTurn = stored.requiredTurnId
      ? historyIncludesAssistantTurn(entries, stored.requiredTurnId)
      : stored.requiredAssistantBaselineIds
        ? historyIncludesNewAssistantMessage(
            entries,
            stored.requiredAssistantBaselineIds,
          )
        : true;
    const fenceSatisfied =
      freshStateVersion === undefined
        ? Array.isArray(body) && containsRequiredTurn
        : freshStateVersion >=
          Math.max(1, positiveStateVersion(stored.stateVersion) ?? 1);
    if (!fenceSatisfied || !containsRequiredTurn) return;
    stored.identityPending = false;
    stored.projectionPending = false;
    stored.projectionStalledAt = undefined;
    stored.requiredAssistantBaselineIds = undefined;
    const receipt = findReceiptByConversation(stored.conversation.id);
    if (receipt)
      retireReceiptAfterMaterialization(receipt.commandId, this.now());
  }

  private newRun(
    onEvent: (event: TurnEvent) => void,
    turnId: string | null = null,
    protocol: "actor" | "workflow" = "actor",
    clientRequestId: string = crypto.randomUUID(),
  ): RunningTurn {
    return {
      clientRequestId,
      protocol,
      turnId,
      stopPendingStart: false,
      streamDispatched: false,
      turnAnnounced: false,
      controller: new AbortController(),
      onEvent,
      cursor: 0,
      currentMessageId: null,
      currentBlockId: null,
      accumulatedText: "",
      finished: false,
      assistantContentObserved: false,
      serverAnswerExpectation: "possible",
      assistantMessageIdsAtDispatch: new Set(),
      sawText: false,
      activityMessageId: null,
      activityBlockCount: 0,
      runBlockId: null,
      runSteps: [],
      stepKeys: new Map(),
      openCards: new Map(),
      promptedApprovalIds: new Set(),
      promptedConnectSlugs: new Map(),
      promptedActionIds: new Map(),
      waitingForApproval: false,
      watchdog: null,
      deliveryStarted: false,
      deliveryTerminal: null,
      deliveryTerminalCount: 0,
      deliveryProtocolError: null,
      actionContinuation: null,
      optimisticMessageAppended: false,
      createRecoveryStarted: false,
      createRecoveryMaterialized: false,
      activeWireTelemetry: null,
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
    if (turnEventPrintsAssistantContent(event)) {
      run.assistantContentObserved = true;
      if (run.activeWireTelemetry) {
        run.activeWireTelemetry.printableTurnEvents += 1;
      }
    }
    let drainActions = false;
    const stored = this.conversations.get(conversationId);
    if (stored) {
      const previousTurnState = stored.turnState;
      const nextTurnState = applyTurnEvent(previousTurnState, event);
      stored.turnState = nextTurnState;
      if (
        event.event === "turn.completed" &&
        nextTurnState !== previousTurnState &&
        run.streamDispatched
      ) {
        stored.lastLocalTurnCompletedAt = this.now();
        stored.lastWireObservationAt = undefined;
        if (
          run.protocol === "workflow" &&
          (stored.identityPending === true ||
            stored.conversation.id.startsWith(WORKFLOW_CONVERSATION_PREFIX))
        ) {
          stored.projectionPending = true;
          stored.requiredTurnId =
            run.serverAnswerExpectation === "none" ? null : run.turnId;
          stored.requiredAssistantBaselineIds =
            run.serverAnswerExpectation === "possible" && !run.turnId
              ? new Set(run.assistantMessageIdsAtDispatch)
              : undefined;
          stored.projectionStalledAt = undefined;
        }
      }
      stored.conversation = {
        ...stored.conversation,
        last_message_at: new Date().toISOString(),
      };
    }
    if (event.event === "turn.completed") {
      run.finished = true;
      this.clearWatchdog(run);
      this.running.delete(conversationId);
      // Safety net for terminals that bypass settleDeliveryTerminal (watchdog
      // stall, abort): an unsettled continuation was never acknowledged, so
      // its reports stay queued rather than leaking as an in-flight batch.
      if (run.actionContinuation && !run.actionContinuation.settled) {
        this.keepActionBatchQueued(conversationId, run);
      }
      if (!run.actionContinuation) {
        this.unblockActionBatches(conversationId);
        drainActions = true;
      } else if (!run.actionContinuation.retryQueued) {
        drainActions = true;
      }
    }
    run.onEvent(event);
    if (drainActions) {
      queueMicrotask(() => this.drainPendingActions(conversationId));
    }
  }

  /**
   * Caller half of the workflow-chat turn contract (`WorkflowChatTurnRequest`
   * server-side; NyxID builds the strict upstream `/api/chat` body from it).
   * A `chatc-…` conversation continues with the last observed `stateVersion`
   * as the read fence; a placeholder (or missing) id starts a new
   * conversation. `commandId` is create-only; Aevatar owns continuation
   * command identities and uses the observed state version as their fence.
   */
  private workflowTurnBody(
    conversationId: string,
    run: RunningTurn,
    prompt: string,
  ): string {
    const stored = this.conversations.get(conversationId);
    const sessionId = this.conversationSessionId(conversationId);
    const serverId = stored?.conversation.id;
    if (serverId && serverId.startsWith(WORKFLOW_CONVERSATION_PREFIX)) {
      const stateVersion = positiveStateVersion(stored.stateVersion);
      if (stateVersion === undefined) {
        throw new AssistantProtocolError(
          "Conversation history is still synchronizing.",
        );
      }
      return JSON.stringify({
        prompt,
        conversationId: serverId,
        minimumStateVersion: stateVersion,
        sessionId,
      });
    }
    return JSON.stringify({
      prompt,
      commandId: run.clientRequestId,
      sessionId,
    });
  }

  /**
   * The conversation's session handle, minted on first use and reused for
   * every later turn. Conversations adopted from the server list have no
   * stored id yet, so this fills one in lazily rather than leaving the field
   * absent for legacy `chatc-…` rows.
   */
  private conversationSessionId(conversationId: string): string {
    const stored = this.conversations.get(conversationId);
    if (!stored) return crypto.randomUUID();
    stored.sessionId ??= crypto.randomUUID();
    return stored.sessionId;
  }

  private latestAssistantTurnId(stored: StoredConversation): string | null {
    for (
      let index = stored.turnState.messages.length - 1;
      index >= 0;
      index -= 1
    ) {
      const message = stored.turnState.messages[index];
      if (message?.role !== "assistant") continue;
      const turnId = safeTurnId(message.turnId);
      if (turnId) return turnId;
    }
    return safeTurnId(stored.requiredTurnId);
  }

  private async readWorkflowHistory(
    conversationId: string,
    signal: AbortSignal,
  ): Promise<{
    readonly body: AevatarHistoryResponse;
    readonly entries: readonly AevatarHistoryEntry[];
    readonly stateVersion: number | undefined;
  }> {
    const body = await assistantApi.get<AevatarHistoryResponse>(
      `${ASSISTANT_PREFIX}/conversations/${conversationId}`,
      signal,
    );
    return {
      body,
      entries: readHistoryEntries(body),
      stateVersion: historyStateVersion(body),
    };
  }

  private async reconcileWorkflowHistory(
    conversationId: string,
    minimumStateVersion: number,
    requiredTurnId: string | null,
    signal: AbortSignal,
  ): Promise<{
    readonly body: AevatarHistoryResponse;
    readonly stateVersion: number;
  } | null> {
    for (const delayMs of HISTORY_RECONCILIATION_DELAYS_MS) {
      await abortableDelay(delayMs, signal);
      try {
        const observation = await this.readWorkflowHistory(
          conversationId,
          signal,
        );
        if (
          observation.stateVersion !== undefined &&
          observation.stateVersion >= minimumStateVersion &&
          historyIncludesAssistantTurn(observation.entries, requiredTurnId)
        ) {
          return {
            body: observation.body,
            stateVersion: observation.stateVersion,
          };
        }
      } catch (error) {
        if (signal.aborted) throw error;
      }
    }
    return null;
  }

  private decodeCreateRecovery(value: AevatarCreateRecoveryResponse): {
    readonly conversationId: string;
    readonly stateVersion: number;
    readonly turnId: string;
  } {
    const conversationId =
      typeof value.conversationId === "string" ? value.conversationId : "";
    const turnId = safeTurnId(value.turnId);
    const rawVersion = value.stateVersion;
    const stateVersion =
      typeof rawVersion === "number"
        ? rawVersion
        : typeof rawVersion === "string"
          ? Number(rawVersion)
          : NaN;
    if (
      !WORKFLOW_SERVER_CONVERSATION_ID_PATTERN.test(conversationId) ||
      !turnId ||
      !Number.isSafeInteger(stateVersion) ||
      stateVersion < 0
    ) {
      throw new AssistantProtocolError(
        "Chat create recovery returned an invalid conversation identity.",
      );
    }
    return { conversationId, stateVersion, turnId };
  }

  private async pollCreateRecovery(
    commandId: string,
    signal: AbortSignal,
  ): Promise<ReturnType<
    AevatarAssistantTransport["decodeCreateRecovery"]
  > | null> {
    for (const delayMs of HISTORY_RECONCILIATION_DELAYS_MS) {
      await abortableDelay(delayMs, signal);
      try {
        const response = await assistantApi.get<AevatarCreateRecoveryResponse>(
          `${ASSISTANT_PREFIX}/conversations/create-recovery/${encodeURIComponent(commandId)}`,
          signal,
        );
        return this.decodeCreateRecovery(response);
      } catch (error) {
        if (signal.aborted) throw error;
        if (error instanceof ApiError && error.status === 404) continue;
        throw error;
      }
    }
    return null;
  }

  private async recoverWorkflowCreate(
    conversationId: string,
    run: RunningTurn,
    signal: AbortSignal,
  ): Promise<boolean> {
    const scopeId = useAuthStore.getState().user?.id;
    if (!scopeId) return false;
    const recovery = await this.pollCreateRecovery(run.clientRequestId, signal);
    if (
      !recovery ||
      useAuthStore.getState().user?.id !== scopeId ||
      this.deletedConversationIds.has(conversationId)
    ) {
      return false;
    }

    const stored = this.conversations.get(conversationId);
    if (!stored) return false;
    const priorId = stored.conversation.id;
    if (
      priorId.startsWith(WORKFLOW_CONVERSATION_PREFIX) &&
      priorId !== recovery.conversationId
    ) {
      throw new AssistantProtocolError(
        "Chat create recovery changed the conversation identity.",
      );
    }

    const reconciled = await this.reconcileWorkflowHistory(
      recovery.conversationId,
      Math.max(1, recovery.stateVersion),
      recovery.turnId,
      signal,
    );
    if (
      !reconciled ||
      useAuthStore.getState().user?.id !== scopeId ||
      this.deletedConversationIds.has(conversationId)
    )
      return false;

    stored.conversation = {
      ...stored.conversation,
      id: recovery.conversationId,
    };
    stored.identityPending = false;
    stored.projectionPending = true;
    stored.requiredTurnId = recovery.turnId;
    stored.stateVersion = Math.max(
      stored.stateVersion ?? 0,
      recovery.stateVersion,
      reconciled.stateVersion,
    );
    stored.createRequest = undefined;
    adoptReceiptIdentity(
      run.clientRequestId,
      recovery.conversationId,
      recovery.stateVersion,
      this.now(),
    );
    this.conversations.set(recovery.conversationId, stored);
    this.conversationAliases.set(conversationId, recovery.conversationId);
    if (this.activeConversationId === conversationId) {
      this.activeConversationId = recovery.conversationId;
    }
    // RUN_STARTED may have supplied a run-actor id from a different identity
    // space. Chat History owns the recovered turn identity.
    run.turnId = recovery.turnId;
    const authoritative = this.applyHistoryResponse(
      recovery.conversationId,
      reconciled.body,
    );
    run.createRecoveryMaterialized = true;
    this.conversations.set(conversationId, authoritative);
    return true;
  }

  private startCreateRecoveryInBackground(
    conversationId: string,
    run: RunningTurn,
  ): void {
    if (run.createRecoveryStarted || run.protocol !== "workflow") return;
    if (!this.workflowCreateNeedsRecovery(conversationId)) return;
    run.createRecoveryStarted = true;
    const recoveryController = this.scopeController();
    void this.recoverWorkflowCreate(
      conversationId,
      run,
      recoveryController.signal,
    )
      .catch(() => undefined)
      .finally(() => this.releaseScopeController(recoveryController));
  }

  private workflowCreateNeedsRecovery(conversationId: string): boolean {
    return Boolean(
      this.conversations
        .get(conversationId)
        ?.conversation.id.startsWith(PENDING_WORKFLOW_CONVERSATION_PREFIX),
    );
  }

  private async streamWorkflowTurn(
    conversationId: string,
    run: RunningTurn,
    prompt: string,
  ): Promise<void> {
    const stored = this.conversations.get(conversationId);
    if (!stored) return;
    const isCreate = !stored.conversation.id.startsWith(
      WORKFLOW_CONVERSATION_PREFIX,
    );

    if (!isCreate && positiveStateVersion(stored.stateVersion) === undefined) {
      const reconciled = await this.reconcileWorkflowHistory(
        stored.conversation.id,
        1,
        this.latestAssistantTurnId(stored),
        run.controller.signal,
      );
      if (run.finished || run.controller.signal.aborted) return;
      if (!reconciled) {
        this.finishTurn(conversationId, run, "failed", {
          code: "history_synchronizing",
          message:
            "Conversation history is still synchronizing. Try again shortly.",
        });
        return;
      }
      this.applyHistoryResponse(stored.conversation.id, reconciled.body);
    }

    if (run.finished || run.controller.signal.aborted) return;
    this.appendOptimisticUserMessage(conversationId, run, prompt);
    let finalFailure = {
      code: "network_error",
      message: "The assistant stream could not be reached. Try again.",
    };
    let reservationDelayIndex = 0;

    while (!run.finished && !run.controller.signal.aborted) {
      this.resetDeliveryState(run);
      const bodyText = this.workflowTurnBody(conversationId, run, prompt);
      const stream = this.startChatStream(
        conversationId,
        run,
        WORKFLOW_CHAT_URL,
        bodyText,
      );
      const response = await stream.headers;
      if (response.kind === "cancelled") return;
      if (response.kind === "network_error") {
        finalFailure = { code: response.code, message: response.message };
        if (isCreate && this.workflowCreateNeedsRecovery(conversationId)) {
          run.createRecoveryStarted = true;
          const recoveryController = this.scopeController();
          try {
            if (
              await this.recoverWorkflowCreate(
                conversationId,
                run,
                recoveryController.signal,
              )
            ) {
              this.settleRecoveredWorkflowCreate(conversationId, run);
              return;
            }
          } catch (error) {
            if (error instanceof AssistantProtocolError) {
              finalFailure = {
                code: "stream_protocol_error",
                message: error.message,
              };
            }
          } finally {
            this.releaseScopeController(recoveryController);
          }
        }
        break;
      }
      if (response.kind === "http_error") {
        const error = streamStartError(response.status, response.body);
        if (
          !isCreate &&
          response.status === 503 &&
          error.code === HISTORY_RESERVATION_UNAVAILABLE &&
          reservationDelayIndex < RESERVATION_RETRY_DELAYS_MS.length
        ) {
          let refreshed = false;
          let refreshFailed = false;
          while (reservationDelayIndex < RESERVATION_RETRY_DELAYS_MS.length) {
            const delayMs = RESERVATION_RETRY_DELAYS_MS[reservationDelayIndex]!;
            reservationDelayIndex += 1;
            await abortableDelay(delayMs, run.controller.signal);
            try {
              const observation = await this.readWorkflowHistory(
                stored.conversation.id,
                run.controller.signal,
              );
              const fence = positiveStateVersion(stored.stateVersion) ?? 1;
              if (
                observation.stateVersion === undefined ||
                observation.stateVersion < fence
              ) {
                continue;
              }
              this.applyHistoryResponse(
                stored.conversation.id,
                observation.body,
              );
              refreshed = true;
              break;
            } catch (refreshError) {
              if (run.controller.signal.aborted) return;
              if (isRetryableHistoryRefreshError(refreshError)) {
                continue;
              }
              finalFailure = {
                code: "history_refresh_failed",
                message:
                  refreshError instanceof Error
                    ? refreshError.message
                    : "Conversation history could not be refreshed.",
              };
              refreshFailed = true;
              break;
            }
          }
          if (refreshed) continue;
          if (refreshFailed) break;
        }
        if (
          isCreate &&
          (response.status === 400 ||
            response.status === 401 ||
            response.status === 403 ||
            response.status === 422)
        ) {
          stored.identityPending = false;
          stored.projectionPending = false;
          deleteReceipt(run.clientRequestId);
        }
        finalFailure = error;
        break;
      }

      const result = await this.consumeTurnStream(conversationId, run, stream);
      if (result.kind === "settled" || run.finished) return;
      finalFailure = result.error;
      if (
        isCreate &&
        this.workflowCreateNeedsRecovery(conversationId) &&
        result.kind === "retryable"
      ) {
        run.createRecoveryStarted = true;
        const recoveryController = this.scopeController();
        try {
          if (
            await this.recoverWorkflowCreate(
              conversationId,
              run,
              recoveryController.signal,
            )
          ) {
            this.settleRecoveredWorkflowCreate(conversationId, run);
            return;
          }
        } catch (error) {
          if (error instanceof AssistantProtocolError) {
            finalFailure = {
              code: "stream_protocol_error",
              message: error.message,
            };
          }
        } finally {
          this.releaseScopeController(recoveryController);
        }
      }
      break;
    }

    if (run.finished || run.controller.signal.aborted) return;
    this.closeOpenMessage(conversationId, run);
    this.finalizeActivity(conversationId, run, "failed");
    this.finishTurn(conversationId, run, "failed", finalFailure);
  }

  private async streamTurn(
    conversationId: string,
    run: RunningTurn,
    prompt: string,
  ): Promise<void> {
    // Serialize behind a previous turn's in-flight stop so this send cannot
    // arrive upstream before the fence commits.
    await this.awaitPendingStop(conversationId);

    if (run.protocol === "workflow") {
      try {
        await this.streamWorkflowTurn(conversationId, run, prompt);
      } catch (error) {
        if (run.finished || run.controller.signal.aborted) return;
        this.finishTurn(conversationId, run, "failed", {
          code:
            error instanceof AssistantProtocolError
              ? "stream_protocol_error"
              : "network_error",
          message:
            error instanceof Error
              ? error.message
              : "The assistant stream could not be reached. Try again.",
        });
      }
      return;
    }

    let finalFailure = {
      code: "network_error",
      message: "The assistant stream could not be reached. Try again.",
    };

    // Actor delivery keeps its existing idempotent replay contract. Workflow
    // retries are handled separately above because their continuation body and
    // reservation-fence semantics differ.
    const target = {
      url: TYPED_CHAT_URL,
      bodyText: JSON.stringify({
        // Aevatar dispatches `/api/chat` on this discriminator; the comparison
        // is ordinal, so the exact lowercase value matters.
        type: "text",
        conversationId,
        prompt,
        clientRequestId: run.clientRequestId,
      }),
    };

    for (let attempt = 0; attempt < STREAM_DELIVERY_ATTEMPTS; attempt += 1) {
      // A cancel can settle the run between attempts (the pre-RUN_STARTED
      // path defers its abort); a finished run must never re-POST.
      if (run.finished || run.controller.signal.aborted) return;
      this.resetDeliveryState(run);
      const stream = this.startChatStream(
        conversationId,
        run,
        target.url,
        target.bodyText,
      );
      const response = await stream.headers;

      if (response.kind === "cancelled") return;
      if (response.kind === "network_error") {
        if (run.finished || run.controller.signal.aborted) return;
        finalFailure = { code: response.code, message: response.message };
        if (attempt + 1 < STREAM_DELIVERY_ATTEMPTS) continue;
        break;
      }
      if (response.kind === "http_error") {
        if (
          RETRYABLE_STREAM_STATUSES.has(response.status) &&
          attempt + 1 < STREAM_DELIVERY_ATTEMPTS
        ) {
          continue;
        }
        finalFailure = streamStartError(response.status, response.body);
        break;
      }

      const result = await this.consumeTurnStream(conversationId, run, stream);
      if (result.kind === "settled" || run.finished) return;
      finalFailure = result.error;
      if (
        result.kind === "retryable" &&
        attempt + 1 < STREAM_DELIVERY_ATTEMPTS
      ) {
        continue;
      }
      break;
    }

    if (run.finished || run.controller.signal.aborted) return;
    this.closeOpenMessage(conversationId, run);
    this.finalizeActivity(conversationId, run, "failed");
    this.finishTurn(conversationId, run, "failed", finalFailure);
  }

  private async streamActionContinuation(
    conversationId: string,
    run: RunningTurn,
    body: ActionContinueBody | ActionWakeBody,
  ): Promise<void> {
    // Action continuations share the actor turn's ordering fence and must not
    // overtake a still-pending stop from the prior turn.
    await this.awaitPendingStop(conversationId);

    const isWake = body.actions.length === 0;
    let finalFailure = {
      code: "network_error",
      message: isWake
        ? "The action wake could not be delivered. Try again."
        : "The action report could not be delivered. It will be retried after the next turn.",
    };
    // This DTO is already rebuilt from the strict allowlist in
    // buildActionContinueBody; no card or model object is spread here.
    const bodyText = JSON.stringify(body);

    for (let attempt = 0; attempt < STREAM_DELIVERY_ATTEMPTS; attempt += 1) {
      if (run.finished || run.controller.signal.aborted) return;
      this.resetDeliveryState(run);
      const stream = this.startChatStream(
        conversationId,
        run,
        TYPED_CHAT_URL,
        bodyText,
      );
      const response = await stream.headers;

      if (response.kind === "cancelled") return;
      if (response.kind === "network_error") {
        if (run.finished || run.controller.signal.aborted) return;
        finalFailure = { ...finalFailure, code: response.code };
        if (attempt + 1 < STREAM_DELIVERY_ATTEMPTS) continue;
        break;
      }
      if (response.kind === "http_error") {
        if (
          RETRYABLE_STREAM_STATUSES.has(response.status) &&
          attempt + 1 < STREAM_DELIVERY_ATTEMPTS
        ) {
          continue;
        }
        finalFailure = streamStartError(response.status, response.body);
        break;
      }

      if (!response.contentType.includes("text/event-stream")) {
        stream.cancel();
        finalFailure = {
          code: "stream_protocol_error",
          message: isWake
            ? "The action wake endpoint did not return an event stream."
            : "The action report endpoint did not return an event stream. Delivery will retry after the next turn.",
        };
        break;
      }

      const result = await this.consumeTurnStream(conversationId, run, stream);
      if (result.kind === "settled" || run.finished) return;
      finalFailure =
        result.error.code === "stream_closed"
          ? {
              code: "stream_closed",
              message: "The action continuation closed before it started.",
            }
          : result.error.code === "network_error" ||
              result.error.code === "worker_error"
            ? { ...finalFailure, code: result.error.code }
            : result.error;
      if (
        result.kind === "retryable" &&
        attempt + 1 < STREAM_DELIVERY_ATTEMPTS
      ) {
        continue;
      }
      break;
    }

    if (run.finished || run.controller.signal.aborted) return;
    this.keepActionBatchQueued(conversationId, run);
    this.closeOpenMessage(conversationId, run);
    this.finalizeActivity(conversationId, run, "failed");
    this.finishTurn(conversationId, run, "failed", finalFailure);
  }

  /**
   * Shared completion handling for initial turns, approval continuations, and
   * action continuations. Fetch, UTF-8 decoding, SSE framing, JSON parsing,
   * and bounded batching happen in the stream worker (or inline fallback).
   */
  private async consumeTurnStream(
    conversationId: string,
    run: RunningTurn,
    stream: ChatStreamRequestHandle,
  ): Promise<StreamConsumptionResult> {
    try {
      this.armWatchdog(conversationId, run);
      const completion = await stream.completion;
      if (run.finished || run.controller.signal.aborted) {
        this.recordStreamTransportOutcome(stream, "cancelled");
        return { kind: "settled" };
      }

      if (run.deliveryProtocolError) {
        stream.cancel();
        this.recordStreamTransportOutcome(
          stream,
          run.deliveryProtocolError.code,
        );
        return { kind: "protocol_error", error: run.deliveryProtocolError };
      }
      if (completion.kind !== "complete") {
        const failure = this.streamCompletionFailure(completion);
        this.recordStreamTransportOutcome(
          stream,
          failure.kind === "settled" ? "cancelled" : failure.error.code,
        );
        return failure;
      }
      if (run.deliveryTerminal) {
        if (
          run.protocol === "workflow" &&
          this.workflowCreateNeedsRecovery(conversationId)
        ) {
          const error =
            run.deliveryTerminal.kind === "error"
              ? run.deliveryTerminal.error
              : {
                  code: "stream_protocol_error",
                  message: "Chat completed without a conversation context.",
                };
          this.recordStreamTransportOutcome(stream, error.code);
          return {
            kind: "retryable",
            error,
          };
        }
        this.recordStreamTransportOutcome(
          stream,
          run.deliveryTerminal.kind === "finished"
            ? run.deliveryTerminal.status
            : run.deliveryTerminal.kind === "error"
              ? run.deliveryTerminal.error.code
              : "cancelled",
        );
        this.settleDeliveryTerminal(conversationId, run, run.deliveryTerminal);
        return { kind: "settled" };
      }

      if (run.waitingForApproval) {
        // EOF at a human gate is a pause, not a truncation: Aevatar may
        // close an idle stream while an approval waits (PRD §3.4). The card
        // stays actionable; the decision starts the continuation stream.
        // Reaching an approval gate proves the continuation turn ran, so its
        // reports are delivered and must not be resent.
        this.acceptActionBatch(conversationId, run);
        this.closeOpenMessage(conversationId, run);
        this.finalizeActivity(conversationId, run, "waiting");
        this.recordStreamTransportOutcome(stream, "completed");
        this.finishTurn(conversationId, run, "completed", null);
        return { kind: "settled" };
      }
      // EOF without RUN_FINISHED / RUN_ERROR is a truncated run (proxy idle
      // kill, upstream drop), not a success. Settle the partial state and
      // report it; the server-side run may still finish, in which case the
      // full reply surfaces on the next history reload.
      this.recordStreamTransportOutcome(stream, "stream_closed");
      return {
        kind: "retryable",
        error: {
          code: "stream_closed",
          message:
            "The stream ended before the assistant finished. The reply may be incomplete; it will appear in full once the conversation reloads.",
        },
      };
    } finally {
      this.clearWatchdog(run);
    }
  }

  private async consumeApprovalContinuation(
    conversationId: string,
    run: RunningTurn,
    stream: ChatStreamRequestHandle,
  ): Promise<void> {
    const result = await this.consumeTurnStream(conversationId, run, stream);
    if (result.kind === "settled" || run.finished) return;
    this.closeOpenMessage(conversationId, run);
    this.finalizeActivity(conversationId, run, "failed");
    this.finishTurn(conversationId, run, "failed", result.error);
  }

  private startChatStream(
    conversationId: string,
    run: RunningTurn,
    url: string,
    bodyText: string,
  ): ChatStreamRequestHandle {
    let stream: ChatStreamRequestHandle | null = null;
    run.streamDispatched = true;
    const { featureEnabled, captureEnabled } =
      useAssistantWireLogStore.getState();
    const wireLogEnabled = featureEnabled && captureEnabled;
    const wireTelemetry: StreamWireTelemetry | null = wireLogEnabled
      ? {
          startedAt: this.now(),
          framesSeen: 0,
          printableFramesSeen: 0,
          printableTurnEvents: 0,
          wireBytes: 0,
          terminalReceived: false,
        }
      : null;
    run.activeWireTelemetry = wireTelemetry;
    const bufferedWireEvents = new Map<string, ChatStreamWireEvent[]>();
    let wireExchangeId: string | null | undefined;
    const onWire = wireLogEnabled
      ? (event: ChatStreamWireEvent) => {
          try {
            if (wireTelemetry) {
              if (event.type === "lines") {
                wireTelemetry.wireBytes += Math.max(0, event.bytes);
              } else if (event.type === "body") {
                wireTelemetry.wireBytes = Math.max(
                  wireTelemetry.wireBytes,
                  event.bytes,
                );
              }
            }
            if (wireExchangeId === null) return;
            if (wireExchangeId !== undefined) {
              attachStreamWireEvent(wireExchangeId, event);
              return;
            }
            const buffered = bufferedWireEvents.get(event.requestId) ?? [];
            bufferedWireEvents.set(event.requestId, [...buffered, event]);
          } catch {
            // Diagnostic capture must not affect live frame delivery.
          }
        }
      : undefined;
    stream = chatStreamClient.start({
      url,
      bodyText,
      signal: run.controller.signal,
      headers: wireLogEnabled
        ? { [DEBUG_UPSTREAM_REQUEST_HEADER]: "1" }
        : undefined,
      onWire,
      onFrames: (frames) => {
        if (wireTelemetry && frames.length > 0) {
          const observedAt = this.now();
          wireTelemetry.framesSeen += frames.length;
          wireTelemetry.firstFrameAt ??= observedAt;
          wireTelemetry.lastFrameAt = observedAt;
        }
        for (const frame of frames) {
          const printableEventsBefore = wireTelemetry?.printableTurnEvents ?? 0;
          this.handleAgUiFrame(conversationId, run, frame);
          if (
            wireTelemetry &&
            wireTelemetry.printableTurnEvents > printableEventsBefore
          ) {
            wireTelemetry.printableFramesSeen += 1;
          }
          if (run.finished || run.deliveryProtocolError) {
            if (run.deliveryProtocolError) stream?.cancel();
            break;
          }
        }
      },
    });
    if (wireTelemetry) this.streamWireTelemetry.set(stream, wireTelemetry);
    if (wireLogEnabled) {
      void stream.headers
        .then((response) => {
          if (response.kind !== "response" && response.kind !== "http_error") {
            wireExchangeId = null;
            if (wireTelemetry) wireTelemetry.exchangeId = null;
            bufferedWireEvents.clear();
            return;
          }
          wireExchangeId = captureAssistantWireLogHeader(
            response.debugUpstream,
            "sse",
            response.status,
          );
          if (!wireExchangeId) {
            if (wireTelemetry) wireTelemetry.exchangeId = null;
            bufferedWireEvents.clear();
            return;
          }
          if (wireTelemetry) {
            wireTelemetry.exchangeId = wireExchangeId;
            this.flushStreamTransportTelemetry(wireTelemetry);
          }
          for (const events of bufferedWireEvents.values()) {
            for (const event of events) {
              attachStreamWireEvent(wireExchangeId, event);
            }
          }
          bufferedWireEvents.clear();
        })
        .catch(() => {
          wireExchangeId = null;
          if (wireTelemetry) wireTelemetry.exchangeId = null;
          bufferedWireEvents.clear();
        });
    }
    return stream;
  }

  private flushStreamTransportTelemetry(telemetry: StreamWireTelemetry): void {
    if (!telemetry.exchangeId || !telemetry.transportOutcome) return;
    useAssistantWireLogStore
      .getState()
      .attachTransportTelemetry(telemetry.exchangeId, {
        transportOutcome: telemetry.transportOutcome,
        framesSeen: telemetry.framesSeen,
        printableFramesSeen: telemetry.printableFramesSeen,
        printableTurnEvents: telemetry.printableTurnEvents,
        wireBytes: telemetry.wireBytes,
        terminalReceived: telemetry.terminalReceived,
        firstFrameMs:
          telemetry.firstFrameAt === undefined
            ? null
            : Math.max(0, telemetry.firstFrameAt - telemetry.startedAt),
        lastFrameMs:
          telemetry.lastFrameAt === undefined
            ? null
            : Math.max(0, telemetry.lastFrameAt - telemetry.startedAt),
      });
  }

  private recordStreamTransportOutcome(
    stream: ChatStreamRequestHandle,
    outcome: string,
  ): void {
    const telemetry = this.streamWireTelemetry.get(stream);
    if (!telemetry) return;
    telemetry.transportOutcome = safeErrorCode(outcome, "unknown");
    this.flushStreamTransportTelemetry(telemetry);
  }

  private recordActiveTransportOutcome(
    run: RunningTurn,
    outcome: string,
  ): void {
    const telemetry = run.activeWireTelemetry;
    if (!telemetry) return;
    telemetry.transportOutcome = safeErrorCode(outcome, "unknown");
    this.flushStreamTransportTelemetry(telemetry);
  }

  private streamCompletionFailure(
    completion: Exclude<ChatStreamCompletionResult, { kind: "complete" }>,
  ): StreamConsumptionResult {
    if (completion.kind === "cancelled") return { kind: "settled" };
    if (completion.kind === "http_error") {
      return {
        kind: "retryable",
        error: streamStartError(completion.status, completion.body),
      };
    }
    return {
      kind: "retryable",
      error: { code: completion.code, message: completion.message },
    };
  }

  private resetDeliveryState(run: RunningTurn): void {
    run.deliveryStarted = false;
    run.deliveryTerminal = null;
    run.deliveryTerminalCount = 0;
    run.deliveryProtocolError = null;
  }

  // -------------------------------------------------------------------------
  // Watchdog (G-hang): keepalives keep the connection alive but are not
  // progress; only real frames re-arm the timer.
  // -------------------------------------------------------------------------

  private armWatchdog(conversationId: string, run: RunningTurn): void {
    this.clearWatchdog(run);
    if (run.finished || run.waitingForApproval) return;
    run.watchdog = setTimeout(() => {
      // A hung run holds the conversation actor: without a server-side stop
      // the next send fails with ACTIVE_TURN_REQUIRES_STEERING until the
      // run reaches its own terminal. Best-effort, like user cancel.
      // Pre-RUN_STARTED this is a no-op BY DESIGN: after a full watchdog
      // period of silence there is no addressable turn identity, and no
      // announcing frame is coming — unlike a user cancel, which defers
      // its abort for a bounded window because the frame may be in flight.
      this.requestServerStop(conversationId, run);
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
    payload: ChatStreamFrame,
  ): void {
    const frame = payload as AgUiFrame;
    if (typeof frame !== "object" || frame === null) return;

    const type = this.frameType(frame);
    if (
      run.activeWireTelemetry &&
      (type === "RUN_FINISHED" ||
        type === "RUN_ERROR" ||
        type === "RUN_STOPPED")
    ) {
      run.activeWireTelemetry.terminalReceived = true;
    }
    const isKeepalive =
      type === "CUSTOM" &&
      frame.custom?.name === "aevatar.nyxid_chat.keepalive";
    if (!isKeepalive) {
      this.armWatchdog(conversationId, run);
    }

    if (
      run.deliveryTerminal &&
      !isKeepalive &&
      type !== "RUN_FINISHED" &&
      type !== "RUN_ERROR" &&
      type !== "RUN_STOPPED"
    ) {
      // The workflow stream legitimately trails its terminal with the
      // projection snapshot and late `aevatar.raw.observed` envelopes;
      // they are not renderable and not an ordering fault.
      if (run.protocol === "workflow") return;
      run.deliveryProtocolError = {
        code: "stream_protocol_error",
        message: "The assistant stream sent data after its terminal frame.",
      };
      return;
    }

    if (type !== "RUN_STARTED" && !isKeepalive && !run.deliveryStarted) {
      // Workflow streams open with `custom aevatar.chat.context` (which
      // starts the delivery below) and `custom aevatar.run.context` BEFORE
      // `runStarted` — pre-start CUSTOM frames are the contract there, not
      // a fault.
      if (run.protocol !== "workflow" || type !== "CUSTOM") {
        run.deliveryProtocolError ??= {
          code: "stream_protocol_error",
          message:
            "The assistant stream sent data before identifying the turn.",
        };
        return;
      }
    }

    switch (type) {
      case "RUN_STARTED": {
        if (run.protocol === "workflow" && run.deliveryStarted) {
          // `aevatar.chat.context` already identified the turn; this frame
          // only contributes the run actor identity (`runId`), which the
          // workflow surface uses for run-level control, not as a turn id.
          return;
        }
        const authoritativeTurnId = safeTurnId(
          frame.turnId ?? frame.runStarted?.turnId ?? frame.runStarted?.runId,
        );
        if (run.deliveryStarted || !authoritativeTurnId) {
          run.deliveryProtocolError ??= {
            code: "stream_protocol_error",
            message: run.deliveryStarted
              ? "The assistant stream started the same delivery more than once."
              : "The assistant stream did not provide a valid turn id.",
          };
          return;
        }
        run.deliveryStarted = true;
        if (run.turnId && run.turnId !== authoritativeTurnId) {
          run.deliveryProtocolError = {
            code: "stream_protocol_error",
            message: "The assistant replay changed the turn id.",
          };
          return;
        }
        if (!run.turnId) run.turnId = authoritativeTurnId;
        if (run.stopPendingStart) {
          // A cancel landed before this frame named the turn. Deliver the
          // stop it was waiting on, then drop the connection — the local
          // turn already settled as cancelled.
          run.stopPendingStart = false;
          const releaseFence = run.resolvePreStartFence;
          run.resolvePreStartFence = undefined;
          let stop: Promise<void> | null = null;
          try {
            stop = this.requestServerStop(conversationId, run);
          } finally {
            if (releaseFence) {
              if (stop) {
                // The placeholder lifts only once the real stop settles
                // (chained on the RAW stop, never the composed map entry —
                // the entry contains the placeholder itself and chaining
                // onto it would deadlock), so a waiter serialized on the
                // fence cannot overtake the fence commit.
                void stop.then(releaseFence);
              } else {
                // The stop never launched (synchronous throw): release the
                // placeholder outright; the composed entry retires itself
                // once every component has settled.
                releaseFence();
              }
            }
            run.controller.abort();
          }
          return;
        }
        if (!run.turnAnnounced) {
          run.turnAnnounced = true;
          this.emit(conversationId, run, {
            cursor: this.nextCursor(run),
            event: "turn.status",
            turn_id: authoritativeTurnId,
            status: "running",
          });
        }
        return;
      }
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
        // Legacy/generic authorization signals are intentionally not blockers.
        // Only `CUSTOM nyxid.authorization.required` carries NyxID's exact
        // credential classification contract.
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
        const error = frame.runError;
        this.recordDeliveryTerminal(run, {
          kind: "error",
          error: {
            code: safeErrorCode(error?.code, "run_error"),
            message: safeErrorMessage(
              error?.message,
              "The assistant run failed.",
            ),
          },
        });
        return;
      }
      case "RUN_FINISHED": {
        const status = frame.runFinished?.status;
        if (
          status !== undefined &&
          status !== "completed" &&
          status !== "blocked"
        ) {
          run.deliveryProtocolError = {
            code: "stream_protocol_error",
            message:
              "The assistant stream returned an unknown terminal status.",
          };
          return;
        }
        // Workflow terminal payload: when nothing streamed (no text deltas
        // and no `RoleChatSessionCompletedEvent` mined en route), the run
        // result is the only carrier of the assistant's reply.
        const output = frame.runFinished?.result?.output;
        if (
          run.protocol === "workflow" &&
          !run.sawText &&
          typeof output === "string" &&
          output.trim()
        ) {
          this.emitStaticText(conversationId, run, output);
        }
        this.recordDeliveryTerminal(run, {
          kind: "finished",
          status: status === "blocked" ? "blocked" : "completed",
        });
        return;
      }
      case "RUN_STOPPED": {
        // Server-side stop (operator, policy, or an upstream cancel) —
        // terminal, and NOT a truncated stream: reporting it as one would
        // tell the user their reply may still be coming when it will not.
        this.recordDeliveryTerminal(run, { kind: "stopped" });
        return;
      }
      case "STEP_STARTED": {
        const name = frame.stepStarted?.stepName ?? "workflow-step";
        this.startRunStep(conversationId, run, name, name);
        return;
      }
      case "STEP_FINISHED": {
        const step = frame.stepFinished ?? {};
        const name = step.stepName ?? "workflow-step";
        this.finishRunStep(
          conversationId,
          run,
          name,
          step.success !== false,
          step.success === false ? "Step failed" : "Completed",
        );
        return;
      }
      case "STATE_SNAPSHOT":
        // Workflow projection state; telemetry only, never rendered.
        return;
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

  /**
   * Frame type, tolerating both `type`-tagged and body-keyed variants.
   * The reference client accepts either shape for every frame family, so
   * the fallbacks below mirror its list exactly — a body-keyed
   * `{runFinished:{}}` that fell through to UNKNOWN would leave the turn
   * looking truncated when it actually completed.
   */
  private frameType(frame: AgUiFrame): string {
    if (frame.type) return frame.type.toUpperCase();
    if (frame.runStarted) return "RUN_STARTED";
    if (frame.runFinished) return "RUN_FINISHED";
    if (frame.runStopped) return "RUN_STOPPED";
    if (frame.runError) return "RUN_ERROR";
    if (frame.stepStarted) return "STEP_STARTED";
    if (frame.stepFinished) return "STEP_FINISHED";
    if (frame.textMessageStart) return "TEXT_MESSAGE_START";
    if (frame.textMessageContent) return "TEXT_MESSAGE_CONTENT";
    if (frame.textMessageEnd) return "TEXT_MESSAGE_END";
    if (frame.toolCallStart) return "TOOL_CALL_START";
    if (frame.toolCallEnd) return "TOOL_CALL_END";
    if (frame.toolApprovalRequest) return "TOOL_APPROVAL_REQUEST";
    if (frame.authorizationRequired) return "AUTHORIZATION_REQUIRED";
    if (frame.usage) return "USAGE";
    if (frame.mediaContent) return "MEDIA_CONTENT";
    if (frame.stateSnapshot) return "STATE_SNAPSHOT";
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
      case "nyxid.authorization.required": {
        const blocker = parseAuthorizationBlocker(custom.payload);
        if (blocker) this.addConnectCard(conversationId, run, blocker);
        return;
      }
      case "nyxid.action.request": {
        const request = assistantActionRequestSchema.safeParse(payload);
        if (request.success) {
          this.addActionCard(
            conversationId,
            run,
            request.data,
            fingerprintStableRequestInput(request.data.params),
          );
        } else {
          const unsupported = recoverUnsupportedAssistantActionRequest(payload);
          if (unsupported) {
            this.addActionCard(
              conversationId,
              run,
              unsupported,
              fingerprintStableRequestInput(payload),
            );
          }
        }
        return;
      }
      case "aevatar.tool_approval.pending":
        this.addApprovalCard(
          conversationId,
          run,
          payload as ToolApprovalPayload,
        );
        return;
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
        if (run.turnId) {
          this.emit(conversationId, run, {
            cursor: this.nextCursor(run),
            event: "turn.status",
            turn_id: run.turnId,
            status: "waiting",
          });
        }
        return;
      case "aevatar.chat.context":
        this.applyWorkflowChatContext(
          conversationId,
          run,
          payload as WorkflowChatContextPayload,
        );
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
  /**
   * First frame of a workflow-chat turn: the chat-history reservation's
   * identity. Starts the delivery (the turn id lives here, not in
   * `runStarted`), records the continuation watermark, and — on a new
   * conversation's first turn — aliases the client placeholder id to the
   * server `chatc-…` id so every later address reaches the server-backed
   * conversation.
   */
  private applyWorkflowChatContext(
    conversationId: string,
    run: RunningTurn,
    payload: WorkflowChatContextPayload,
  ): void {
    if (run.protocol !== "workflow") return;

    const stored = this.conversations.get(conversationId);
    const activeScopeId = useAuthStore.getState().user?.id;
    // The console always knows its active scope. NyxID can receive a stream
    // before the auth store hydrates, so enforce the comparison only when the
    // local user id is known pending a live production capture.
    if (
      activeScopeId &&
      (typeof payload.scopeId !== "string" || payload.scopeId !== activeScopeId)
    ) {
      run.deliveryProtocolError = {
        code: "stream_protocol_error",
        message: "The assistant stream returned a context for another scope.",
      };
      return;
    }
    const serverId =
      typeof payload.conversationId === "string" &&
      WORKFLOW_SERVER_CONVERSATION_ID_PATTERN.test(payload.conversationId)
        ? payload.conversationId
        : null;
    const priorId = stored?.conversation.id;
    // Fail closed when the create turn never names its server conversation:
    // accepting the turn would leave the record on its `workflow-pending-…`
    // placeholder, so the NEXT send would build another create body and mint
    // a SECOND upstream conversation instead of continuing this one.
    if (
      !serverId &&
      priorId !== undefined &&
      !priorId.startsWith(WORKFLOW_CONVERSATION_PREFIX)
    ) {
      run.deliveryProtocolError ??= {
        code: "stream_protocol_error",
        message:
          "The assistant stream did not provide a valid conversation id.",
      };
      return;
    }
    // A replay must resolve to the conversation it already adopted. Silently
    // re-keying would orphan the first server row while later events stay
    // attributed to the old identity.
    if (
      serverId &&
      priorId !== undefined &&
      priorId !== serverId &&
      priorId.startsWith(WORKFLOW_CONVERSATION_PREFIX)
    ) {
      run.deliveryProtocolError = {
        code: "stream_protocol_error",
        message: "The assistant replay changed the conversation id.",
      };
      return;
    }
    if (stored && serverId && stored.conversation.id !== serverId) {
      const commandId = stored.createRequest?.commandId ?? run.clientRequestId;
      stored.conversation = { ...stored.conversation, id: serverId };
      stored.identityPending = false;
      stored.createRequest = undefined;
      this.conversations.set(serverId, stored);
      this.conversationAliases.set(conversationId, serverId);
      adoptReceiptIdentity(commandId, serverId, undefined, this.now());
      if (this.activeConversationId === conversationId) {
        this.activeConversationId = serverId;
      }
    }

    const rawVersion = payload.stateVersion;
    const stateVersion =
      typeof rawVersion === "number" ? rawVersion : Number(rawVersion);
    if (!Number.isSafeInteger(stateVersion) || stateVersion < 0) {
      run.deliveryProtocolError = {
        code: "stream_protocol_error",
        message: "The assistant stream returned an invalid state version.",
      };
      return;
    }
    if (stored && stateVersion > 0) {
      stored.stateVersion = Math.max(stored.stateVersion ?? 0, stateVersion);
      advanceReceiptFence(serverId ?? stored.conversation.id, stateVersion);
    }

    if (run.deliveryStarted) return;
    const turnId = safeTurnId(payload.turnId);
    if (!turnId) {
      run.deliveryProtocolError = {
        code: "stream_protocol_error",
        message: "The assistant stream did not provide a valid turn id.",
      };
      return;
    }
    // Same replay guard as the conversation id: a retry that comes back on a
    // DIFFERENT turn is not the turn this run is streaming, and attributing
    // its events to the original turn id would interleave two turns.
    if (run.turnId && run.turnId !== turnId) {
      run.deliveryProtocolError = {
        code: "stream_protocol_error",
        message: "The assistant replay changed the turn id.",
      };
      return;
    }
    if (stored) stored.requiredTurnId = turnId;
    run.deliveryStarted = true;
    run.turnId ??= turnId;
    if (!run.turnAnnounced) {
      run.turnAnnounced = true;
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "turn.status",
        turn_id: turnId,
        status: "running",
      });
    }
  }

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
    this.conversations
      .get(conversationId)
      ?.activityMessageTurnIds.set(messageId, run.turnId);
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

  /**
   * Park the tool step behind a pending approval. Falls back to the last
   * still-active step when the frame names no toolCallId/stepId — the common
   * live sequence is TOOL_CALL_START directly followed by the approval
   * request, and without this the ledger spins forever on a decided card.
   */
  private markStepWaiting(
    conversationId: string,
    run: RunningTurn,
    key: string,
    approvalRequestId: string,
  ): void {
    let step: RunStepState | undefined;
    if (key) {
      const index = run.stepKeys.get(key);
      step = index === undefined ? undefined : run.runSteps[index];
    } else {
      step = [...run.runSteps]
        .reverse()
        .find((candidate) => candidate.status === "active");
    }
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
  }

  // -------------------------------------------------------------------------
  // Cards and artifacts.
  // -------------------------------------------------------------------------

  private addActionCard(
    conversationId: string,
    run: RunningTurn,
    request: AssistantActionRequest,
    requestFingerprint: string,
  ): void {
    const resolved = resolveAssistantAction(request);
    const stored = this.conversations.get(conversationId);
    const existing = stored?.turnState.messages
      .flatMap((message) => message.blocks)
      .find(
        (block): block is ActionCardContentBlock =>
          block.type === "action_card" &&
          block.action_request_id === request.actionRequestId,
      );
    const knownBlockId =
      run.promptedActionIds.get(request.actionRequestId) ?? existing?.block_id;
    if (knownBlockId) {
      run.promptedActionIds.set(request.actionRequestId, knownBlockId);
      if (!existing) return;
      const terminal =
        existing.status === "completed" ||
        existing.status === "conflicted" ||
        existing.status === "declined" ||
        existing.status === "failed";
      if (
        !matchesCommittedActionRequest(
          existing,
          request,
          resolved.params,
          stored?.actionRequestFingerprints.get(request.actionRequestId),
          requestFingerprint,
        )
      ) {
        if (terminal) return;
        this.emit(conversationId, run, {
          cursor: this.nextCursor(run),
          event: "block.updated",
          block_id: knownBlockId,
          patch: {
            status: "conflicted",
            outcome_note: ACTION_REQUEST_CONFLICT_NOTE,
          },
        });
        return;
      }
      stored?.actionRequestFingerprints.set(
        request.actionRequestId,
        requestFingerprint,
      );
      if (existing.status === "blocked") {
        const nextStatus = resolved.supported ? "pending" : "unsupported";
        this.emit(conversationId, run, {
          cursor: this.nextCursor(run),
          event: "block.updated",
          block_id: knownBlockId,
          patch: {
            status: nextStatus,
            outcome_note:
              nextStatus === "unsupported"
                ? composeBlockedUnsupportedNote(existing.outcome_note)
                : "",
          },
        });
        return;
      }
      if (terminal || resolved.supported || existing.status === "unsupported") {
        return;
      }
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.updated",
        block_id: knownBlockId,
        patch: {
          status: "unsupported",
          outcome_note: "",
        },
      });
      return;
    }

    const block: ActionCardContentBlock = {
      type: "action_card",
      block_id: newId("action-card"),
      action: request.action,
      action_request_id: request.actionRequestId,
      origin_turn_id: request.originTurnId,
      actor_id: request.actorId || undefined,
      task_id: request.taskId,
      step_id: request.stepId,
      params: resolved.params,
      status: resolved.supported ? "pending" : "unsupported",
      outcome_note: "",
    };
    this.appendActivityBlock(conversationId, run, block);
    run.promptedActionIds.set(request.actionRequestId, block.block_id);
    stored?.actionRequestFingerprints.set(
      request.actionRequestId,
      requestFingerprint,
    );
    // Deliberately excluded from openCards: this browser action remains
    // interactive after the origin run reaches its normal terminal frame.
  }

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
    this.markStepWaiting(
      conversationId,
      run,
      payload.toolCallId ?? payload.stepId ?? "",
      requestId,
    );
    if (run.runBlockId) this.patchRunBlock(conversationId, run);
    if (run.turnId) {
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "turn.status",
        turn_id: run.turnId,
        status: "waiting",
      });
    }
  }

  private addConnectCard(
    conversationId: string,
    run: RunningTurn,
    blocker: AuthorizationBlocker,
  ): void {
    const rawSlug = blocker.serviceSlug;
    const dedupeKey = rawSlug;
    const serviceName = blocker.serviceLabel;
    const message = blocker.safeMessage;

    // One card per missing service. Replayed typed events update the same card
    // instead of appending duplicate recovery actions.
    const existingCardId = run.promptedConnectSlugs.get(dedupeKey);
    if (existingCardId) {
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.updated",
        block_id: existingCardId,
        patch: {
          service_name: serviceName,
          reason_code: blocker.reasonCode,
          steps: [
            {
              title:
                blocker.reasonCode === "NYXID_UNAUTHORIZED"
                  ? `Reconnect ${serviceName}`
                  : `Connect ${serviceName}`,
              body: message,
              done: false,
            },
          ],
        },
      });
      return;
    }
    const block: ConnectCardContentBlock = {
      type: "connect_card",
      block_id: newId("connect-card"),
      // Raw NyxID service slug, exactly as the AI Services page passes it
      // to ServiceIcon — the glyph registry is keyed by full slugs
      // (`api-github`, `llm-openai`), so stripping a prefix breaks icons.
      catalog_slug: rawSlug || "custom",
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
          title:
            blocker.reasonCode === "NYXID_UNAUTHORIZED"
              ? `Reconnect ${serviceName}`
              : `Connect ${serviceName}`,
          body: message,
          done: false,
        },
      ],
      footer: "Brokered by NyxID · configure in AI Services, then ask again",
      reason_code: blocker.reasonCode,
    };
    this.appendActivityBlock(conversationId, run, block);
    run.promptedConnectSlugs.set(dedupeKey, block.block_id);
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

  private recordDeliveryTerminal(
    run: RunningTurn,
    terminal: NonNullable<RunningTurn["deliveryTerminal"]>,
  ): void {
    run.deliveryTerminalCount += 1;
    if (run.deliveryTerminalCount !== 1) {
      run.deliveryProtocolError = {
        code: "stream_protocol_error",
        message: "The assistant stream sent more than one terminal frame.",
      };
      return;
    }
    run.deliveryTerminal = terminal;
  }

  private settleDeliveryTerminal(
    conversationId: string,
    run: RunningTurn,
    terminal: NonNullable<RunningTurn["deliveryTerminal"]>,
  ): void {
    this.closeOpenMessage(conversationId, run);
    if (run.actionContinuation) {
      // A rejected continuation is published as a `nyxid.continuation.changed`
      // CUSTOM frame on the *origin* turn's session, never as a reason code on
      // this stream — the client only ever sees a generic terminal error (or a
      // stall that trips the watchdog). So any error terminal means "the
      // server may never have admitted these reports": requeue instead of
      // dropping them. Only a real terminal (RUN_FINISHED / RUN_STOPPED)
      // proves the continuation turn ran.
      if (terminal.kind === "error") {
        this.keepActionBatchQueued(conversationId, run);
      } else {
        this.acceptActionBatch(conversationId, run);
      }
    }
    switch (terminal.kind) {
      case "error":
        this.finalizeActivity(conversationId, run, "failed");
        this.finishTurn(conversationId, run, "failed", terminal.error);
        return;
      case "stopped":
        this.finalizeActivity(conversationId, run, "cancelled");
        if (run.turnId) {
          this.emit(conversationId, run, {
            cursor: this.nextCursor(run),
            event: "turn.status",
            turn_id: run.turnId,
            status: "cancelled",
          });
        }
        this.finishTurn(conversationId, run, "cancelled", null);
        return;
      case "finished":
        if (terminal.status === "blocked") {
          this.finalizeActivity(conversationId, run, "blocked");
          if (
            !run.assistantContentObserved &&
            !run.createRecoveryMaterialized
          ) {
            run.serverAnswerExpectation = "none";
          }
          this.finishTurn(conversationId, run, "blocked", null);
        } else {
          this.finalizeActivity(
            conversationId,
            run,
            run.waitingForApproval ? "waiting" : "done",
          );
          if (
            !run.assistantContentObserved &&
            !run.createRecoveryMaterialized
          ) {
            run.serverAnswerExpectation = "none";
          }
          this.finishTurn(conversationId, run, "completed", null);
        }
    }
  }

  private settleRecoveredWorkflowCreate(
    conversationId: string,
    run: RunningTurn,
  ): void {
    if (run.deliveryTerminal) {
      this.settleDeliveryTerminal(conversationId, run, run.deliveryTerminal);
      return;
    }
    // Header failures and truncated streams have no terminal kind to retain;
    // the reconciled History row is the authoritative successful completion.
    this.finishTurn(conversationId, run, "completed", null);
  }

  private closeOpenMessage(conversationId: string, run: RunningTurn): void {
    if (!run.currentMessageId || !run.currentBlockId) return;
    const messageId = run.currentMessageId;
    const blocks = textToBlocks(run.accumulatedText, messageId);
    const [leadingBlock] = blocks;

    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.completed",
      block_id: run.currentBlockId,
      block:
        leadingBlock?.type === "text"
          ? { ...leadingBlock, block_id: run.currentBlockId }
          : { type: "text", block_id: run.currentBlockId, text: "" },
    });

    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "message.completed",
      message_id: messageId,
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
   * - `blocked`   — a typed authorization gate is terminal for the turn;
   *                 the connection card remains actionable.
   * - `failed`    — active steps fail; undecided approvals cancel; connect
   *                 cards stay actionable (they ARE the recovery path).
   * - `cancelled` — PRD §5.6: every open block reaches a terminal state.
   */
  private finalizeActivity(
    conversationId: string,
    run: RunningTurn,
    outcome: "done" | "waiting" | "blocked" | "failed" | "cancelled",
  ): void {
    if (run.runBlockId) {
      for (const step of run.runSteps) {
        if (step.status === "active") {
          step.status =
            outcome === "done"
              ? "done"
              : outcome === "failed" || outcome === "blocked"
                ? "failed"
                : outcome === "cancelled"
                  ? "skipped"
                  : step.status;
          if (step.status === "done" && step.meta === "Running") {
            step.meta = "Completed";
          }
        } else if (
          step.status === "waiting" &&
          (outcome === "blocked" ||
            outcome === "failed" ||
            outcome === "cancelled")
        ) {
          // A terminal run must not carry a non-terminal step: an approval
          // that will never be decided (run died / user stopped) is skipped.
          step.status = "skipped";
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
            : outcome === "blocked"
              ? "awaiting_connection"
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
        (outcome === "blocked" && kind === "approval") ||
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
    status: "blocked" | "completed" | "failed" | "cancelled",
    error: { code: string; message: string } | null,
  ): void {
    if (run.finished) return;
    this.recordActiveTransportOutcome(run, error?.code ?? status);
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
    this.acceptActionBatch(conversationId, run);
    this.closeOpenMessage(conversationId, run);
    this.finalizeActivity(conversationId, run, "waiting");
    this.finishTurn(conversationId, run, "completed", null);
    run.controller.abort();
  }

  /**
   * Stop flow: settles the local turn per the PRD stop-flow — every open
   * block reaches a terminal state (§5.6) — and fires a best-effort `:stop`
   * control command so Aevatar commits a stop fence instead of running the
   * turn to its own terminal. When the server has already announced the
   * turn, the fetch aborts immediately and the stop goes out with that
   * turnId. When cancel lands BEFORE RUN_STARTED, the reader is kept alive
   * (bounded) so the announcing frame can still deliver the turnId the stop
   * needs; the RUN_STARTED handler then stops and aborts. The stop is
   * 202-accepted and asynchronous upstream; if it fails or never fires, the
   * pre-existing behavior stands (the run finishes server-side and surfaces
   * on the next history reload).
   */
  private cancelTurn(conversationId: string, run: RunningTurn): void {
    if (run.finished) return;
    if (run.actionContinuation) {
      this.keepActionBatchQueued(conversationId, run);
    }
    if (run.protocol === "workflow") {
      // Workflow has no stop control. Once a create request was dispatched,
      // recover its Chat History identity even if RUN_STARTED already supplied
      // a run-actor id; cancellation cannot prove the create was rejected.
      if (run.streamDispatched) {
        this.startCreateRecoveryInBackground(conversationId, run);
      } else {
        const stored = this.conversations.get(conversationId);
        if (stored?.identityPending) {
          stored.identityPending = false;
          deleteReceipt(run.clientRequestId);
        }
      }
      run.controller.abort();
    } else if (run.turnId) {
      run.controller.abort();
      this.requestServerStop(conversationId, run);
    } else if (!run.streamDispatched) {
      // The stream request never left the client (e.g. the send is still
      // queued behind an earlier turn's stop fence): nothing reached
      // upstream, so cancel is purely local. Installing a placeholder here
      // would OVERWRITE that earlier fence and let a later send overtake
      // the still-pending stop.
      run.controller.abort();
    } else {
      run.stopPendingStart = true;
      // Install the fence NOW: the stop request cannot exist until
      // RUN_STARTED names the turn, but a follow-up send or delete must
      // already serialize behind the eventual stop. Lifted when the
      // deferred stop settles, or when the window expires without a turn.
      // trackFence COMPOSES with any live entry instead of replacing it.
      const fence = new Promise<void>((resolve) => {
        run.resolvePreStartFence = resolve;
      });
      this.trackFence(conversationId, fence);
      setTimeout(() => {
        if (run.stopPendingStart) {
          // No RUN_STARTED inside the window: nothing to stop. Resolve
          // only — the composed map entry retires itself once every
          // component has settled.
          run.stopPendingStart = false;
          run.resolvePreStartFence?.();
          run.resolvePreStartFence = undefined;
        }
        if (!run.controller.signal.aborted) run.controller.abort();
      }, PRE_START_STOP_WINDOW_MS);
    }
    this.clearWatchdog(run);
    if (run.currentBlockId) {
      const messageId = run.currentMessageId;
      const [leadingBlock] = textToBlocks(
        run.accumulatedText,
        messageId ?? run.currentBlockId,
      );
      const stored = this.conversations.get(conversationId);
      const openBlock = stored?.turnState.messages
        .flatMap((message) => message.blocks)
        .find((block) => block.block_id === run.currentBlockId);
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.completed",
        block_id: run.currentBlockId,
        block:
          leadingBlock?.type === "text"
            ? { ...leadingBlock, block_id: run.currentBlockId }
            : openBlock
              ? toTerminalBlock(openBlock)
              : { type: "text", block_id: run.currentBlockId, text: "" },
      });
      if (messageId) {
        this.emit(conversationId, run, {
          cursor: this.nextCursor(run),
          event: "message.completed",
          message_id: messageId,
        });
      }
      run.currentMessageId = null;
      run.currentBlockId = null;
      run.accumulatedText = "";
    }
    this.finalizeActivity(conversationId, run, "cancelled");
    if (run.turnId) {
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "turn.status",
        turn_id: run.turnId,
        status: "cancelled",
      });
    }
    this.finishTurn(conversationId, run, "cancelled", null);
  }

  /**
   * Best-effort server-side stop (the `feature/integrate` `:stop` control
   * contract): a fresh `stopRequestId` per intent keeps the command
   * idempotent upstream, and `expectedStateVersion: 0` skips the
   * optimistic-concurrency fence — the transport does not track actor
   * state versions. Requires the server-announced `turnId` (a
   * pre-RUN_STARTED cancel defers here via `stopPendingStart`). Failures
   * are swallowed — stop is an upgrade over the previous client-only
   * cancel, never a new failure mode — but the in-flight request is
   * tracked in `pendingStops` so follow-up sends and deletes serialize
   * behind the fence.
   */
  private requestServerStop(
    conversationId: string,
    run: RunningTurn,
  ): Promise<void> | null {
    // The workflow surface still stops elsewhere (`runs/{runId}:stop`), so
    // only typed assistant conversations use the canonical chat command here.
    if (run.protocol === "workflow") return null;
    if (!run.turnId) return null;
    const actorConversationId = this.canonicalConversationId(conversationId);
    // Own deadline: a server that accepts but never answers must not pin
    // the pendingStops entry (and tax every later send with the full fence
    // wait). A manual controller instead of AbortSignal.timeout so this
    // never throws on an environment that lacks the static — the deferred
    // placeholder release depends on this call not throwing.
    const deadline = this.scopeController();
    const deadlineTimer = setTimeout(
      () => deadline.abort(),
      STOP_REQUEST_DEADLINE_MS,
    );
    const pending = apiClient<unknown>(`${ASSISTANT_PREFIX}/chat`, {
      method: "POST",
      body: {
        type: "task.stop",
        conversationId: actorConversationId,
        turnId: run.turnId,
        stopRequestId: crypto.randomUUID(),
        clientRequestId: crypto.randomUUID(),
        expectedStateVersion: 0,
      },
      preserveSessionOn401: true,
      signal: deadline.signal,
      ...assistantWireLogOptions(),
    }).then(
      () => {
        clearTimeout(deadlineTimer);
        this.releaseScopeController(deadline);
      },
      () => {
        clearTimeout(deadlineTimer);
        this.releaseScopeController(deadline);
      },
    );
    this.trackFence(actorConversationId, pending);
    return pending;
  }

  /**
   * Register a fence component for the conversation, COMPOSING with any
   * live entry rather than replacing it — no control path may drop a
   * still-pending fence someone else could be relying on. Every component
   * is self-bounded, so the composition is too; the entry retires itself
   * once everything it covers has settled.
   */
  private trackFence(conversationId: string, component: Promise<void>): void {
    const prior = this.pendingStops.get(conversationId);
    const tracked = prior
      ? Promise.all([prior, component]).then(() => undefined)
      : component;
    this.pendingStops.set(conversationId, tracked);
    void tracked.then(() => {
      if (this.pendingStops.get(conversationId) === tracked) {
        this.pendingStops.delete(conversationId);
      }
    });
  }

  /**
   * Wait for this conversation's in-flight `:stop` fence before the next
   * send or delete goes out. Without this, a fast follow-up can reach
   * Aevatar ahead of the stop — request ordering across HTTP connections
   * is not guaranteed — and fail with ACTIVE_TURN_REQUIRES_STEERING.
   *
   * Awaited DIRECTLY, no outer race: every tracked promise is
   * self-bounded — a real stop by its STOP_REQUEST_DEADLINE_MS abort, a
   * pre-start placeholder by the PRE_START_STOP_WINDOW_MS expiry (plus,
   * when RUN_STARTED lands late in the window, the chained stop's own
   * deadline). An outer bound shorter than the placeholder lifetime would
   * reopen the exact overtake the fence exists to prevent.
   */
  private async awaitPendingStop(conversationId: string): Promise<void> {
    const pending = this.pendingStops.get(conversationId);
    if (!pending) return;
    await pending;
  }
}
