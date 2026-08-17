import { ApiError, apiClient } from "@/lib/api-client";
import {
  ACTION_REQUEST_CONFLICT_NOTE,
  composeBlockedUnsupportedNote,
  composeUnreportedCompletedNote,
} from "@/lib/assistant/action-notes";
import {
  ASSISTANT_SESSION_EXPIRED_MESSAGE,
  ASSISTANT_UPSTREAM_AUTH_MESSAGE,
  AssistantConversationNotFoundError,
  AssistantProtocolError,
  AssistantTurnActiveError,
  AssistantTurnCancelledError,
  isNyxIdSessionAuthFailure,
} from "@/lib/assistant/errors";
import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import {
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
  assistantInputRequestSchema,
  buildInputResolveBody,
  inputAnswerSchema,
  type AssistantInputRequest,
  type InputAnswer,
} from "@/schemas/assistant-input";
import {
  applyTurnEvent,
  EMPTY_TURN_STATE,
  toTerminalBlock,
} from "@/lib/assistant/stream";
import {
  applyCurrentTaskState,
  createTaskProjection,
  reduceTaskFrame,
  taskCan,
  type AssistantTaskProjection,
} from "@/lib/assistant/task-state";
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
  InputCardContentBlock,
  RunContentBlock,
  TaskPlanContentBlock,
  TaskStep,
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
  post<T>(endpoint: string, body?: unknown, signal?: AbortSignal): Promise<T> {
    return apiClient<T>(endpoint, {
      method: "POST",
      body,
      preserveSessionOn401: true,
      signal,
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

// Every browser mutation uses the typed NyxIdChat command surface. Legacy
// chatc-* rows remain addressable only through history and delete resources.
const TYPED_CHAT_URL = "/api/v1/assistant/chat";

// Client-local placeholder minted before the typed surface returned its
// authoritative actor. New chats no longer mint one; it survives only so a
// stale `?c=nyxid-pending-…` URL from a pre-studio session still resolves to
// a clean not-found instead of a network error.
export const AEVATAR_LEGACY_PENDING_CONVERSATION_PREFIX = "nyxid-pending-";

// Legacy Studio conversation ids retained for historical read/delete only.
export const AEVATAR_LEGACY_CONVERSATION_PREFIX = "chatc-";

// Client-local id for a typed conversation before authoritative RUN_STARTED
// adoption. A reload forgets the placeholder and lists the server row instead.
export const AEVATAR_DRAFT_CONVERSATION_PREFIX = "draft-";

export const AEVATAR_TYPED_CONVERSATION_PREFIX = "nyxid-chat-";

export function isLegacyConversationId(id: string): boolean {
  return id.startsWith(AEVATAR_LEGACY_CONVERSATION_PREFIX);
}

function escapeRegexLiteral(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const TYPED_SERVER_CONVERSATION_ID_PATTERN = new RegExp(
  `^${escapeRegexLiteral(AEVATAR_TYPED_CONVERSATION_PREFIX)}[0-9a-f]{32}$`,
);

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
const DECISION_OBSERVATION_DELAYS_MS = [0, 250, 750, 1_500] as const;

// Hard deadline on the composite DELETE. The deletion reservation rejects
// sends and approvals while it holds, so an unanswered DELETE without a
// bound would lock the conversation permanently.
const DELETE_REQUEST_DEADLINE_MS = 15_000;

// Inline media larger than this (base64 chars ≈ 6 MB decoded) is summarized
// as text instead of being embedded as a data: URL artifact.
const MAX_MEDIA_DATA_CHARS = 8_000_000;

const MAX_TOOL_SUMMARY_CHARS = 160;

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
  readonly presentation?: {
    readonly action?: string;
    readonly target?: string;
    readonly actorLabel?: string;
    readonly reversibility?: string;
    readonly grantBoundary?: string;
  };
}

type AuthorizationReasonCode =
  | "NYXID_SERVICE_NOT_CONNECTED"
  | "NYXID_UNAUTHORIZED";

export interface AuthorizationBlocker {
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

interface CustomEnvelope {
  readonly name?: string;
  readonly payload?: unknown;
}

interface AgUiFrame {
  readonly type?: string;
  /** Actor-owned progress sequence. This is not the committed state version. */
  readonly sequence?: string | number;
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
    readonly threadId?: string;
  };
  readonly runFinished?: {
    readonly runId?: string;
    readonly status?: string;
  };
  readonly runStopped?: { readonly reason?: string };
  readonly runError?: { readonly code?: string; readonly message?: string };
  readonly stepStarted?: { readonly stepName?: string };
  readonly stepFinished?: {
    readonly stepName?: string;
    readonly success?: boolean;
  };
  /** Generic AG-UI snapshot frame; typed state comes from actor custom facts. */
  readonly stateSnapshot?: unknown;
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
 * `stateVersion` is the transcript read model's materialization watermark. It
 * is captured when present and never required; legacy-array responses simply
 * leave the stored watermark unchanged.
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
  /** Actor-owned task projection shared by live CUSTOM and `/state`. */
  taskProjection?: AssistantTaskProjection;
  /** Epoch milliseconds of the latest turn.completed applied to this mirror. */
  lastLocalTurnCompletedAt?: number;
  /** Latest materialization watermark observed from the transcript read. */
  stateVersion?: number;
  /** A locally terminal typed turn is not yet confirmed in wire history. */
  projectionPending?: boolean;
  /** Latest local typed turn that must appear before materialization. */
  requiredTurnId?: string | null;
  /** Fallback fence when an interrupted delivery never announced a turn id. */
  requiredAssistantBaselineIds?: Set<string>;
  /** Reconciliation deadline, after which the mirror is explicitly stalled. */
  projectionStalledAt?: number;
  /** One-shot handoff from a cold transcript 404 to the reconciler. */
  lastWireObservationAt?: number;
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
  /** This transport accepts only the typed NyxIdChat actor protocol. */
  readonly protocol: "actor";
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
  /** Tool or actor step identity to its index in `runSteps`. */
  stepKeys: Map<string, number>;
  /** Open cards: block_id → kind, completed at turn finalization. */
  openCards: Map<string, "approval" | "connect" | "input">;
  /** Dedupe guards, per reference client behavior. */
  promptedApprovalIds: Set<string>;
  /** Service dedupe key → connect card block_id (for in-place upgrades). */
  promptedConnectSlugs: Map<string, string>;
  /** Action request id → action card block id. */
  promptedActionIds: Map<string, string>;
  waitingForApproval: boolean;
  /** Exact approval request currently owning the human gate. */
  pendingApprovalRequestId: string | null;
  /** Exact typed input request currently owning the human gate. */
  pendingInputRequestId: string | null;
  /** Suspended on a typed input request while the browser waits for the user. */
  awaitingSignal: boolean;
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
  activeWireTelemetry: StreamWireTelemetry | null;
}

type ReconcileOrigin = "post_terminal" | "cold_observed" | "explicit_retry";

interface ReconcileEntry {
  readonly promise: Promise<ProjectionReconcileOutcome>;
  readonly settle: (outcome: ProjectionReconcileOutcome) => void;
  readonly scopeId: string | null;
  conversationId: string;
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

/** Read an optional protobuf JSON string without inventing a value. */
function protoString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/** Decode a positive protobuf JSON integer into the browser-safe range. */
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

function fingerprintActionRequest(request: AssistantActionRequest): string {
  const params = resolveAssistantAction(request).params;
  if (params.variant === "service_reauthorize") {
    return fingerprintStableRequestInput({
      userServiceId: params.user_service_id,
      requestedScopes: [...params.requested_scopes].sort(),
    });
  }
  if (params.variant === "key_create") {
    return fingerprintStableRequestInput({
      name: params.name,
      platform: params.platform,
      allowedServiceIds: [...params.allowed_service_ids].sort(),
    });
  }
  if (params.variant === "key_rotate") {
    return fingerprintStableRequestInput({ keyId: params.key_id });
  }
  return fingerprintStableRequestInput(request.params);
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

function sameStringSet(
  left: readonly string[],
  right: readonly string[],
): boolean {
  if (left.length !== right.length) return false;
  const sortedLeft = [...left].sort();
  const sortedRight = [...right].sort();
  return sortedLeft.every((value, index) => value === sortedRight[index]);
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
    case "service_reauthorize":
      return (
        right.variant === "service_reauthorize" &&
        left.user_service_id === right.user_service_id &&
        sameStringSet(left.requested_scopes, right.requested_scopes)
      );
    case "key_create":
      return (
        right.variant === "key_create" &&
        left.name === right.name &&
        left.platform === right.platform &&
        sameStringSet(left.allowed_service_ids, right.allowed_service_ids)
      );
    case "key_rotate":
      return right.variant === "key_rotate" && left.key_id === right.key_id;
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
 * `http_<status>`. A 401/403 is attributed to whichever side actually
 * rejected it (see `isNyxIdSessionAuthFailure`); this raw fetch never touches
 * auth state, so it cannot trigger a sign-out on its own.
 */
function streamStartError(
  status: number,
  bodyText: string,
): { code: string; message: string } {
  interface ErrorEnvelope {
    readonly error?: unknown;
    readonly code?: unknown;
    readonly message?: unknown;
    readonly error_code?: unknown;
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
      message: isNyxIdSessionAuthFailure(envelope?.error_code)
        ? ASSISTANT_SESSION_EXPIRED_MESSAGE
        : ASSISTANT_UPSTREAM_AUTH_MESSAGE,
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

/**
 * Strict compatibility decoder for a NyxID readiness blocker embedded in a
 * tool's JSON result:
 *
 *   {"blocked":true,"service_slug":"api-github",
 *    "readiness_status":"ServiceRegistrationRequired",
 *    "reason_code":"USER_SERVICE_NOT_VISIBLE","safe_message":"..."}
 *
 * Keyed on the shape rather than the tool name, so the whole DTO must be present
 * before `blocked` is treated as authority. `blocked: true` plus a slug is not
 * enough: any tool could emit those two keys and mint a bogus connect card
 * while failing its own step. Upstream only sets `blocked` alongside
 * `ServiceRegistrationRequired` with a non-empty code and message, so
 * requiring all five mirrors its own invariant and keeps a transient
 * `NYXID_SOURCE_UNAVAILABLE` an ordinary failure.
 */
const REQUIRE_SERVICE_BLOCKED_STATUS = "ServiceRegistrationRequired";

export function parseToolResultBlocker(
  result: unknown,
): AuthorizationBlocker | null {
  if (typeof result !== "string" || !result.includes("blocked")) return null;
  let record: unknown;
  try {
    record = JSON.parse(result);
  } catch {
    return null;
  }
  if (!record || typeof record !== "object" || Array.isArray(record)) {
    return null;
  }
  const body = record as Record<string, unknown>;
  if (body["blocked"] !== true) return null;
  if (body["readiness_status"] !== REQUIRE_SERVICE_BLOCKED_STATUS) return null;
  // Upstream refuses to set `blocked` without both of these, so their absence
  // means this is not the readiness DTO.
  const rawReason = body["reason_code"];
  const rawSafeMessage = body["safe_message"];
  if (typeof rawReason !== "string" || !rawReason.trim()) return null;
  if (typeof rawSafeMessage !== "string" || !rawSafeMessage.trim()) return null;

  const rawSlug = body["service_slug"];
  if (typeof rawSlug !== "string") return null;
  const serviceSlug = rawSlug.trim().toLowerCase();
  // Upstream normalizes slugs to `[a-z0-9._-]`; keep parity so a legitimate
  // dotted or underscored slug still raises a card.
  if (!/^[a-z0-9][a-z0-9._-]{0,127}$/.test(serviceSlug)) return null;

  // Upstream's `safe_message` here is operator diagnostics ("No caller-visible
  // NyxID UserService matches the requested service."), not user-facing prose,
  // so the card keeps NyxID's actionable wording. `reason_code` is upstream's
  // own vocabulary (`USER_SERVICE_NOT_VISIBLE`), which is a not-connected
  // condition; only an explicit unauthorized code means "reconnect".
  const reasonCode: AuthorizationReasonCode =
    rawReason === "NYXID_UNAUTHORIZED"
      ? "NYXID_UNAUTHORIZED"
      : "NYXID_SERVICE_NOT_CONNECTED";
  const serviceLabel = humanizeSlug(serviceSlug);
  return {
    serviceSlug,
    serviceLabel,
    reasonCode,
    safeMessage:
      reasonCode === "NYXID_UNAUTHORIZED"
        ? `Reconnect ${serviceLabel} to continue.`
        : `Connect ${serviceLabel} to continue.`,
  };
}

/**
 * An artifact's `download_url` is rendered directly as an anchor `href`, so a
 * producer-supplied location is only safe after its scheme is checked — a
 * `javascript:` or `data:text/html` URI would otherwise be one click away.
 * Anything not plainly fetchable is dropped, which degrades to the shapeless
 * media acknowledgement rather than rendering a live link.
 */
function safeMediaUrl(value: string): string | undefined {
  if (!value) return undefined;
  let parsed: URL;
  try {
    parsed = new URL(value, window.location.origin);
  } catch {
    return undefined;
  }
  return parsed.protocol === "https:" || parsed.protocol === "http:"
    ? parsed.toString()
    : undefined;
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
 * becomes `artifact`. Reasoning frames are acknowledged but never rendered.
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
    this.activeConversationId = null;
    this.listFetchedAt = 0;
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
      for (const entry of response.conversations ?? []) {
        const id = entry?.id?.trim();
        if (id) this.mergeIndexEntry(id, entry, new Set());
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
    // A typed first turn creates the actor. The local draft id exists only so
    // navigation and optimistic state have a key before RUN_STARTED publishes
    // the authoritative nyxid-chat-* identity.
    const createdAt = new Date().toISOString();
    const conversation: Conversation = {
      id: `${AEVATAR_DRAFT_CONVERSATION_PREFIX}${crypto.randomUUID()}`,
      title: "New chat",
      created_at: createdAt,
      last_message_at: createdAt,
    };
    this.conversations.set(conversation.id, {
      conversation,
      turnState: EMPTY_TURN_STATE,
      actionRequestFingerprints: new Map(),
      activityMessageTurnIds: new Map(),
      taskProjection: undefined,
    });
    this.activeConversationId = conversation.id;
    return conversation;
  }

  async deleteConversation(conversationId: string): Promise<void> {
    this.ensureScope();
    const requestedId = conversationId;
    const aliasedConversationId = this.conversationAliases.get(conversationId);
    if (
      conversationId.startsWith(AEVATAR_DRAFT_CONVERSATION_PREFIX) &&
      !aliasedConversationId
    ) {
      const run = this.running.get(conversationId);
      if (run) this.cancelTurn(conversationId, run);
      this.tombstoneConversation(conversationId);
      return;
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

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    const scopeId = this.ensureScope();
    const requestedId = conversationId;
    conversationId = this.canonicalConversationId(conversationId);
    if (this.deletedConversationIds.has(conversationId)) {
      throw new AssistantConversationNotFoundError();
    }
    const existing = this.conversations.get(conversationId);
    if (
      !existing &&
      (conversationId.startsWith(AEVATAR_LEGACY_PENDING_CONVERSATION_PREFIX) ||
        conversationId.startsWith(AEVATAR_DRAFT_CONVERSATION_PREFIX))
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
      this.activateConversation(conversationId);
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
      existing.projectionPending &&
      existing.projectionStalledAt === undefined &&
      (existing.turnState.messages.length > 0 ||
        existing.requiredTurnId != null ||
        existing.lastLocalTurnCompletedAt !== undefined)
    ) {
      this.activateConversation(conversationId);
      return this.historyFromStored(existing);
    }
    // A pending placeholder exists nowhere server-side — the id never left
    // this client — so the read can only 404 and land in the
    // `noServerTranscriptYet` fallback below. Serve the local mirror without
    // the round trip.
    if (
      existing &&
      (conversationId.startsWith(AEVATAR_LEGACY_PENDING_CONVERSATION_PREFIX) ||
        conversationId.startsWith(AEVATAR_DRAFT_CONVERSATION_PREFIX))
    ) {
      this.activateConversation(conversationId);
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
          if (!conversationId.startsWith(AEVATAR_LEGACY_CONVERSATION_PREFIX)) {
            throw new AssistantConversationNotFoundError();
          }
          const membership = await this.fetchRawIndexMembership(conversationId);
          if (scopeId !== this.ensureScope()) {
            throw new AssistantConversationNotFoundError();
          }
          if (membership === "unavailable") throw error;
          if (membership !== true) {
            throw new AssistantConversationNotFoundError();
          }
          stored =
            this.conversations.get(conversationId) ??
            this.syntheticPendingConversation(conversationId, {
              projectionPending: true,
            });
          stored.projectionPending = true;
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
          conversationId.startsWith(AEVATAR_LEGACY_CONVERSATION_PREFIX) &&
          existing.turnState.messages.length === 0 &&
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
    if (TYPED_SERVER_CONVERSATION_ID_PATTERN.test(conversationId)) {
      await this.hydrateTaskState(conversationId, stored);
      if (
        scopeId !== this.ensureScope() ||
        this.deletedConversationIds.has(conversationId)
      ) {
        throw new AssistantConversationNotFoundError();
      }
    }
    this.activateConversation(conversationId);
    return this.historyFromStored(stored);
  }

  private activateConversation(conversationId: string): void {
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
        : !turnActive && stored.projectionPending
          ? { awaitingProjection: true }
          : {}),
    };
  }

  private syntheticPendingConversation(
    conversationId: string,
    facts: Pick<StoredConversation, "projectionPending" | "stateVersion">,
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
      taskProjection: undefined,
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
      for (const entry of entries) {
        const id = entry?.id?.trim();
        if (id) this.mergeIndexEntry(id, entry, new Set());
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
    const conversationId = this.canonicalConversationId(requestedId);
    const stored = this.conversations.get(conversationId);
    if (conversationId.startsWith(AEVATAR_DRAFT_CONVERSATION_PREFIX)) {
      return Promise.resolve({ status: "timed_out", conversationId });
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
      stored.projectionPending = true;
    }
    const policy = PROJECTION_BACKOFF_POLICY;
    const createdAt = this.now();
    const coldObservationIsCurrent =
      stored?.lastWireObservationAt !== undefined &&
      createdAt - stored.lastWireObservationAt >= 0 &&
      createdAt - stored.lastWireObservationAt <=
        PROJECTION_BACKOFF_POLICY.floorMs;
    const origin: ReconcileOrigin = wasStalled
      ? "explicit_retry"
      : coldObservationIsCurrent
        ? "cold_observed"
        : "post_terminal";
    if (origin === "cold_observed" && stored) {
      stored.lastWireObservationAt = undefined;
    }
    const startsImmediately = origin === "explicit_retry";
    const initialAttempt = origin === "cold_observed" ? 1 : 0;
    let settle!: (outcome: ProjectionReconcileOutcome) => void;
    const promise = new Promise<ProjectionReconcileOutcome>((resolve) => {
      settle = resolve;
    });
    const entry: ReconcileEntry = {
      promise,
      settle,
      scopeId,
      conversationId,
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
    const turnInFlight = this.running.has(entry.conversationId);
    if (
      activeStored &&
      (turnInFlight || isTurnActive(activeStored.turnState.activeTurn?.status))
    ) {
      entry.deadlineAt = this.now() + PROJECTION_BACKOFF_POLICY.deadlineMs;
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
      try {
        const body = await assistantApi.get<AevatarHistoryResponse>(
          `${ASSISTANT_PREFIX}/conversations/${entry.conversationId}`,
          controller.signal,
        );
        if (entry.scopeId !== this.ownerScopeId) return;
        const postFetchStored = this.conversations.get(entry.conversationId);
        const postFetchTurnInFlight = this.running.has(entry.conversationId);
        if (
          postFetchStored &&
          (postFetchTurnInFlight ||
            isTurnActive(postFetchStored.turnState.activeTurn?.status))
        ) {
          entry.deadlineAt = this.now() + PROJECTION_BACKOFF_POLICY.deadlineMs;
          rescheduleAfterTurn = true;
          return;
        }
        const projected = this.applyHistoryResponse(entry.conversationId, body);
        if (TYPED_SERVER_CONVERSATION_ID_PATTERN.test(entry.conversationId)) {
          await this.hydrateTaskState(entry.conversationId, projected);
        }
        if (!projected.projectionPending) {
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

      entry.attempt += 1;
      const deadlineReached =
        entry.finalObservationDue || this.now() >= entry.deadlineAt;
      entry.finalObservationDue = false;
      if (
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
        const membership =
          observedMembership ??
          (await this.fetchRawIndexMembership(
            entry.conversationId,
            controller.signal,
          ));
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
      entry.nextAttemptAt =
        this.now() +
        nextBackoffDelay(
          PROJECTION_BACKOFF_POLICY,
          Math.max(0, entry.attempt - 1),
          this.random,
        );
    }
    this.resumeReconcileEntry(entry);
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
      isTurnActive(stored.turnState.activeTurn?.status) ||
      (stored.taskProjection?.task?.status === "active" &&
        !this.pendingStops.has(conversationId))
    ) {
      throw new AssistantTurnActiveError();
    }
    const normalized = content.trim();
    if (!normalized || normalized.length > MAX_MESSAGE_CHARS) {
      throw new Error("Message must contain between 1 and 32768 characters.");
    }

    if (isLegacyConversationId(stored.conversation.id)) {
      throw new AssistantProtocolError(
        "Legacy conversations are read-only. Start a new chat to continue with the typed assistant.",
      );
    }
    if (
      !stored.conversation.id.startsWith(AEVATAR_DRAFT_CONVERSATION_PREFIX) &&
      !TYPED_SERVER_CONVERSATION_ID_PATTERN.test(stored.conversation.id)
    ) {
      throw new AssistantProtocolError(
        "The conversation does not have a valid typed assistant identity.",
      );
    }
    const run = this.newRun(onEvent, null, "actor", crypto.randomUUID());
    run.assistantMessageIdsAtDispatch = new Set(
      stored.turnState.messages
        .filter((message) => message.role === "assistant")
        .map((message) => message.id),
    );
    this.running.set(conversationId, run);
    this.appendOptimisticUserMessage(conversationId, run, normalized);
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

  async stopTask(conversationId: string): Promise<void> {
    this.ensureScope();
    const requestedId = conversationId;
    conversationId = this.canonicalConversationId(conversationId);
    this.assertTypedTaskConversation(conversationId);
    if (this.deletingConversations.has(conversationId)) {
      throw new Error("This conversation is being deleted.");
    }
    const controller = this.scopeController();
    const operation = this.dispatchTaskStop(
      conversationId,
      crypto.randomUUID(),
      controller.signal,
    );
    this.trackFence(
      conversationId,
      operation.then(
        () => undefined,
        () => undefined,
      ),
    );
    try {
      await operation;
      const run =
        this.running.get(requestedId) ?? this.running.get(conversationId);
      if (run) {
        this.closeOpenMessage(conversationId, run);
        this.finalizeActivity(conversationId, run, "cancelled");
        this.finishTurn(conversationId, run, "cancelled", null);
        run.controller.abort();
      }
    } finally {
      controller.abort();
      this.releaseScopeController(controller);
    }
  }

  async steerTask(conversationId: string, instruction: string): Promise<void> {
    this.ensureScope();
    conversationId = this.canonicalConversationId(conversationId);
    this.assertTypedTaskConversation(conversationId);
    const normalized = instruction.trim();
    if (!normalized || normalized.length > MAX_MESSAGE_CHARS) {
      throw new Error("Steering must contain between 1 and 32768 characters.");
    }
    const controller = this.scopeController();
    try {
      await this.awaitPendingStop(conversationId);
      const projection = await this.readTaskControlProjection(
        conversationId,
        controller.signal,
      );
      const turnId = protoString(projection.activeTurn?.["turnId"]);
      if (!turnId) {
        throw new AssistantProtocolError(
          "The assistant no longer has active work to steer.",
        );
      }
      await assistantApi.post(
        `${ASSISTANT_PREFIX}/chat`,
        {
          type: "task.steer",
          conversationId,
          turnId,
          steeringId: crypto.randomUUID(),
          clientRequestId: crypto.randomUUID(),
          instruction: normalized,
          expectedStateVersion: projection.stateVersion,
        },
        controller.signal,
      );
    } finally {
      controller.abort();
      this.releaseScopeController(controller);
    }
  }

  async retryStep(conversationId: string, stepId: string): Promise<void> {
    await this.dispatchStepControl(conversationId, stepId, "retry");
  }

  async skipStep(conversationId: string, stepId: string): Promise<void> {
    await this.dispatchStepControl(conversationId, stepId, "skip");
  }

  async resolvePlan(
    conversationId: string,
    blockId: string,
    confirmed: boolean,
  ): Promise<void> {
    this.ensureScope();
    conversationId = this.canonicalConversationId(conversationId);
    this.assertTypedTaskConversation(conversationId);
    if (this.deletingConversations.has(conversationId)) {
      throw new Error("This conversation is being deleted.");
    }
    const stored = this.conversations.get(conversationId);
    const card = stored?.turnState.messages
      .flatMap((message) => message.blocks)
      .find(
        (block): block is TaskPlanContentBlock =>
          block.type === "task_plan" && block.block_id === blockId,
      );
    const clickedGate = card?.plan.gate;
    if (
      !card ||
      clickedGate?.mode !== "confirm" ||
      clickedGate.status !== "pending" ||
      !clickedGate.requestId ||
      !clickedGate.taskId ||
      !clickedGate.planId ||
      clickedGate.planRevision === undefined
    ) {
      throw new AssistantProtocolError("This plan gate is no longer pending.");
    }

    const controller = this.scopeController();
    try {
      await this.awaitPendingStop(conversationId);
      const projection = await this.readTaskControlProjection(
        conversationId,
        controller.signal,
      );
      const gate = projection.task?.gate;
      if (
        gate?.mode !== "confirm" ||
        gate.status !== "pending" ||
        gate.requestId !== clickedGate.requestId ||
        gate.taskId !== clickedGate.taskId ||
        gate.planId !== clickedGate.planId ||
        gate.planRevision !== clickedGate.planRevision
      ) {
        throw new AssistantProtocolError(
          "The actor no longer offers this exact plan gate.",
        );
      }
      await assistantApi.post<{ readonly status: string }>(
        `${ASSISTANT_PREFIX}/chat`,
        {
          type: "plan.resolve",
          conversationId,
          taskId: gate.taskId,
          planId: gate.planId,
          requestId: gate.requestId,
          clientRequestId: crypto.randomUUID(),
          planRevision: gate.planRevision,
          confirmed,
          expectedStateVersion: projection.stateVersion,
        },
        controller.signal,
      );

      // JSON 202 means accepted for actor dispatch, not committed. Refresh the
      // same projection once and let only actor-owned state settle the gate.
      try {
        await this.readTaskControlProjection(conversationId, controller.signal);
      } catch (error) {
        if (controller.signal.aborted) throw error;
      }
    } finally {
      controller.abort();
      this.releaseScopeController(controller);
    }
  }

  /** Submit a version-fenced approval decision and project JSON acceptance. */
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
    // approval.resolve addresses a typed nyxid-chat actor. A legacy chatc id
    // is historical display identity only and must never reach this command.
    if (isLegacyConversationId(conversationId)) {
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
    const requestId = card?.approval_request_id;
    if (!stored || !card || !requestId) {
      throw new Error("Approval request was not found.");
    }
    if (card.decision !== null) {
      throw new Error("This approval was already decided.");
    }
    const run = this.reserveHumanDecision(
      conversationId,
      "approval",
      blockId,
      requestId,
      onEvent ?? noopEvent,
    );
    try {
      await this.awaitPendingStop(conversationId);
      this.throwIfControlCancelled(run);
      const preflight = await this.readDecisionPreflight(
        conversationId,
        "approval",
        requestId,
        run.controller.signal,
      );
      if (preflight.committed) {
        this.applyCommittedApproval(
          conversationId,
          blockId,
          preflight.approved!,
          preflight.stateVersion,
          onEvent ?? noopEvent,
        );
        return null;
      }
      const expectedStateVersion = preflight.stateVersion;
      this.throwIfControlCancelled(run);
      await assistantApi.post<{ readonly status: string }>(
        `${ASSISTANT_PREFIX}/chat`,
        {
          type: "approval.resolve",
          conversationId,
          clientRequestId: crypto.randomUUID(),
          requestId,
          approved,
          expectedStateVersion,
        },
        run.controller.signal,
      );
      this.throwIfControlCancelled(run);

      this.emitLocalBlockPatch(
        conversationId,
        blockId,
        {
          decision_submission: approved ? "approved" : "denied",
          state_version: expectedStateVersion,
        },
        onEvent ?? noopEvent,
      );
      const committedStateVersion = await this.observeDecisionCommit(
        conversationId,
        "approval",
        requestId,
        expectedStateVersion,
        approved,
        run.controller.signal,
      );
      if (committedStateVersion === null) {
        this.emitLocalBlockPatch(
          conversationId,
          blockId,
          { decision_submission: null },
          onEvent ?? noopEvent,
        );
        throw new AssistantProtocolError(
          "The approval decision was accepted for dispatch, but its committed result was not observed. The card is retryable and current state will be checked before another submission.",
        );
      }
      this.applyCommittedApproval(
        conversationId,
        blockId,
        approved,
        committedStateVersion,
        onEvent ?? noopEvent,
      );
      return null;
    } catch (error) {
      if (run.controller.signal.aborted) {
        throw new AssistantTurnCancelledError();
      }
      throw error;
    } finally {
      this.releaseHumanDecision(conversationId, run);
    }
  }

  /** Submit a strict input answer against the exact committed request version. */
  async resolveInput(
    conversationId: string,
    blockId: string,
    answer: InputAnswer,
    onEvent?: (event: TurnEvent) => void,
  ): Promise<TurnHandle | null> {
    this.ensureScope();
    conversationId = this.canonicalConversationId(conversationId);
    if (this.deletingConversations.has(conversationId)) {
      throw new Error("This conversation is being deleted.");
    }
    if (!TYPED_SERVER_CONVERSATION_ID_PATTERN.test(conversationId)) {
      throw new AssistantProtocolError(
        "Input answers require a typed assistant conversation.",
      );
    }
    const stored = this.conversations.get(conversationId);
    const card = stored?.turnState.messages
      .flatMap((message) => message.blocks)
      .find(
        (block): block is InputCardContentBlock =>
          block.type === "input_card" && block.block_id === blockId,
      );
    const requestId = card?.request_id;
    if (!stored || !card || !requestId) {
      throw new Error("Input request was not found.");
    }
    if (card.status !== "pending") {
      throw new Error("This input request was already resolved.");
    }
    const parsedAnswer = inputAnswerSchema.parse(answer);
    const allowFreeText = card.allow_free_text;
    if ("freeText" in parsedAnswer && !allowFreeText) {
      throw new AssistantProtocolError(
        "This input request does not allow a free-text answer.",
      );
    }
    if ("selectedOptionIds" in parsedAnswer) {
      const optionIds = new Set(card.options.map((option) => option.option_id));
      if (
        parsedAnswer.selectedOptionIds.some(
          (optionId) => !optionIds.has(optionId),
        )
      ) {
        throw new AssistantProtocolError(
          "The answer selected an option that is not part of this request.",
        );
      }
      const multiSelect = card.multi_select;
      if (!multiSelect && parsedAnswer.selectedOptionIds.length !== 1) {
        throw new AssistantProtocolError(
          "This input request accepts exactly one selected option.",
        );
      }
    }
    const run = this.reserveHumanDecision(
      conversationId,
      "input",
      blockId,
      requestId,
      onEvent ?? noopEvent,
    );
    try {
      await this.awaitPendingStop(conversationId);
      this.throwIfControlCancelled(run);
      const preflight = await this.readDecisionPreflight(
        conversationId,
        "input",
        requestId,
        run.controller.signal,
      );
      if (preflight.committed) {
        this.emitLocalBlockPatch(
          conversationId,
          blockId,
          { status: "resolved", state_version: preflight.stateVersion },
          onEvent ?? noopEvent,
        );
        return null;
      }
      const expectedStateVersion = preflight.stateVersion;
      const body = buildInputResolveBody(
        conversationId,
        crypto.randomUUID(),
        requestId,
        parsedAnswer,
        expectedStateVersion,
      );
      this.throwIfControlCancelled(run);
      await assistantApi.post<{ readonly status: string }>(
        `${ASSISTANT_PREFIX}/chat`,
        body,
        run.controller.signal,
      );
      this.throwIfControlCancelled(run);
      this.emitLocalBlockPatch(
        conversationId,
        blockId,
        { status: "submitted", state_version: expectedStateVersion },
        onEvent ?? noopEvent,
      );
      const committedStateVersion = await this.observeDecisionCommit(
        conversationId,
        "input",
        requestId,
        expectedStateVersion,
        undefined,
        run.controller.signal,
      );
      if (committedStateVersion !== null) {
        this.emitLocalBlockPatch(
          conversationId,
          blockId,
          { status: "resolved", state_version: committedStateVersion },
          onEvent ?? noopEvent,
        );
      } else {
        this.emitLocalBlockPatch(
          conversationId,
          blockId,
          { status: "pending" },
          onEvent ?? noopEvent,
        );
        throw new AssistantProtocolError(
          "The input answer was accepted for dispatch, but its committed result was not observed. The card is retryable and current state will be checked before another submission.",
        );
      }
      return null;
    } catch (error) {
      if (run.controller.signal.aborted) {
        throw new AssistantTurnCancelledError();
      }
      throw error;
    } finally {
      this.releaseHumanDecision(conversationId, run);
    }
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
      (stored && !isLegacyConversationId(stored.conversation.id)
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
    patch: Partial<ContentBlock>,
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
      return "Connected. Telling the assistant it can use this service…";
    }
    if (disposition === "declined") {
      return "You declined. Nothing was connected and no credential was shared. Telling the assistant…";
    }
    return "The connection did not complete. Telling the assistant…";
  }

  private settledActionOutcomeNote(
    disposition: ActionReport["disposition"],
    delivered: boolean,
  ): string {
    if (disposition === "completed") {
      return delivered
        ? "Connected. The assistant can use this service now."
        : "Connected, but the assistant has not been told yet — NyxID will retry after your next message.";
    }
    if (disposition === "declined") {
      return delivered
        ? "You declined. Nothing was connected and no credential was shared."
        : "You declined. The assistant has not been told yet — NyxID will retry after your next message. No credential was shared.";
    }
    return delivered
      ? "The connection did not complete. Ask the assistant to request this service again."
      : "The connection did not complete, and the assistant has not been told yet — NyxID will retry after your next message.";
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

    // The action frame carries its owning typed actor identity. Never
    // substitute a legacy `chatc-*` history identity.
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
      taskProjection: existing?.taskProjection,
      lastLocalTurnCompletedAt: existing?.lastLocalTurnCompletedAt,
      stateVersion: existing?.stateVersion,
      projectionPending: existing?.projectionPending,
      requiredTurnId: existing?.requiredTurnId,
      requiredAssistantBaselineIds: existing?.requiredAssistantBaselineIds,
      projectionStalledAt: existing?.projectionStalledAt,
      lastWireObservationAt: existing?.lastWireObservationAt,
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

  private async hydrateTaskState(
    conversationId: string,
    stored: StoredConversation,
  ): Promise<void> {
    let response: unknown;
    try {
      response = await assistantApi.get<unknown>(
        `${ASSISTANT_PREFIX}/conversations/${encodeURIComponent(conversationId)}/state`,
      );
    } catch (error) {
      if (!(error instanceof ApiError && error.status === 404)) throw error;
      // Older typed conversations can have a valid transcript while their
      // actor current-state projection is unavailable. Keep the richer local
      // live/history mirror in that case; treating 404 as an authoritative
      // empty snapshot would erase input, approval, and action cards.
      if (stored.turnState.messages.length > 0) return;
      response = { status: "not_found" };
    }
    const current = applyCurrentTaskState(
      stored.taskProjection ?? createTaskProjection(conversationId),
      response,
    );
    if (current.reload) {
      throw new AssistantProtocolError(
        "The assistant requested an unconditional state reload for an unconditional request.",
      );
    }
    stored.taskProjection = current.projection;
    if (current.projection.stateVersion > 0) {
      stored.stateVersion = Math.max(
        stored.stateVersion ?? 0,
        current.projection.stateVersion,
      );
    }
    this.materializeCurrentStateBlocks(stored);
  }

  private assertTypedTaskConversation(conversationId: string): void {
    if (!TYPED_SERVER_CONVERSATION_ID_PATTERN.test(conversationId)) {
      throw new AssistantProtocolError(
        "Task controls require a typed assistant conversation.",
      );
    }
  }

  private async readTaskControlProjection(
    conversationId: string,
    signal: AbortSignal,
  ): Promise<AssistantTaskProjection> {
    const stored = this.conversations.get(conversationId);
    if (!stored) throw new AssistantConversationNotFoundError();
    const response = await assistantApi.get<unknown>(
      `${ASSISTANT_PREFIX}/conversations/${encodeURIComponent(conversationId)}/state`,
      signal,
    );
    const current = applyCurrentTaskState(
      stored.taskProjection ?? createTaskProjection(conversationId),
      response,
    );
    if (
      current.reload ||
      current.projection.stateVersion <= 0 ||
      current.projection.actorId !== conversationId
    ) {
      throw new AssistantProtocolError(
        "The assistant state is not current. Refresh the conversation and try again.",
      );
    }
    stored.taskProjection = current.projection;
    stored.stateVersion = Math.max(
      stored.stateVersion ?? 0,
      current.projection.stateVersion,
    );
    this.materializeCurrentStateBlocks(stored);
    return current.projection;
  }

  private async dispatchTaskStop(
    conversationId: string,
    stopRequestId: string,
    signal: AbortSignal,
    expectedTurnId?: string,
  ): Promise<void> {
    const projection = await this.readTaskControlProjection(
      conversationId,
      signal,
    );
    const turnId = protoString(projection.activeTurn?.["turnId"]);
    if (
      !turnId ||
      (expectedTurnId && expectedTurnId !== turnId) ||
      !taskCan(projection, "stop")
    ) {
      throw new AssistantProtocolError(
        "The actor no longer offers Stop for this task version.",
      );
    }
    await assistantApi.post(
      `${ASSISTANT_PREFIX}/chat`,
      {
        type: "task.stop",
        conversationId,
        turnId,
        stopRequestId,
        clientRequestId: crypto.randomUUID(),
        expectedStateVersion: projection.stateVersion,
      },
      signal,
    );
  }

  private async dispatchStepControl(
    conversationId: string,
    stepId: string,
    action: "retry" | "skip",
  ): Promise<void> {
    this.ensureScope();
    conversationId = this.canonicalConversationId(conversationId);
    this.assertTypedTaskConversation(conversationId);
    if (this.deletingConversations.has(conversationId)) {
      throw new Error("This conversation is being deleted.");
    }
    const controller = this.scopeController();
    try {
      const projection = await this.readTaskControlProjection(
        conversationId,
        controller.signal,
      );
      const step: TaskStep | undefined = projection.steps.get(stepId);
      const turnId =
        step?.operation?.turnId ??
        protoString(projection.activeTurn?.["turnId"]);
      const taskId = step?.operation?.taskId ?? projection.task?.taskId;
      const operationGeneration = step?.operation?.operationGeneration;
      if (
        !step ||
        !turnId ||
        !taskId ||
        !Number.isSafeInteger(operationGeneration) ||
        (operationGeneration ?? 0) <= 0 ||
        !taskCan(projection, action, stepId)
      ) {
        throw new AssistantProtocolError(
          `The actor no longer offers ${action === "retry" ? "Retry" : "Skip"} for this step version.`,
        );
      }
      const requestId = crypto.randomUUID();
      await assistantApi.post(
        `${ASSISTANT_PREFIX}/chat`,
        action === "retry"
          ? {
              type: "step.retry",
              conversationId,
              turnId,
              taskId,
              stepId,
              retryRequestId: requestId,
              clientRequestId: crypto.randomUUID(),
              expectedOperationGeneration: operationGeneration,
              expectedStateVersion: projection.stateVersion,
            }
          : {
              type: "step.skip",
              conversationId,
              turnId,
              taskId,
              stepId,
              skipRequestId: requestId,
              clientRequestId: crypto.randomUUID(),
              expectedOperationGeneration: operationGeneration,
              expectedStateVersion: projection.stateVersion,
            },
        controller.signal,
      );
    } finally {
      controller.abort();
      this.releaseScopeController(controller);
    }
  }

  private materializeCurrentStateBlocks(stored: StoredConversation): void {
    const projection = stored.taskProjection;
    if (!projection) return;
    const existingBlocks = stored.turnState.messages.flatMap(
      (message) => message.blocks,
    );
    const blocks: ContentBlock[] = [];
    if (projection.task) {
      blocks.push({
        type: "task_plan",
        block_id: `current-task-plan:${projection.task.taskId}`,
        state_version: projection.stateVersion,
        progress_sequence: projection.progressSequence,
        plan: projection.task,
      });
    }

    if (projection.pendingInput) {
      const request = assistantInputRequestSchema.safeParse(
        projection.pendingInput,
      );
      if (request.success) {
        blocks.push(
          this.inputCardBlock(
            request.data,
            projection.stateVersion,
            `current-input:${request.data.requestId}`,
          ),
        );
      } else {
        const requestId = protoString(projection.pendingInput["requestId"]);
        const existing = existingBlocks.find(
          (block): block is InputCardContentBlock =>
            block.type === "input_card" && block.request_id === requestId,
        );
        if (existing) {
          blocks.push({
            ...existing,
            state_version: projection.stateVersion,
          });
        }
      }
    } else if (projection.latestInputResolution?.["outcome"] === "accepted") {
      const requestId = protoString(
        projection.latestInputResolution["requestId"],
      );
      const existing = existingBlocks.find(
        (block): block is InputCardContentBlock =>
          block.type === "input_card" && block.request_id === requestId,
      );
      if (existing) {
        blocks.push({
          ...existing,
          status: "resolved",
          state_version: projection.stateVersion,
        });
      }
    }
    if (projection.pendingApproval) {
      const pending = projection.pendingApproval;
      const presentation =
        pending["presentation"] &&
        typeof pending["presentation"] === "object" &&
        !Array.isArray(pending["presentation"])
          ? (pending["presentation"] as ToolApprovalPayload["presentation"])
          : undefined;
      const payload = {
        ...(pending as ToolApprovalPayload),
        ...(presentation ?? {}),
        presentation,
      };
      const requestId =
        payload.requestId ?? payload.approvalRequestId ?? payload.commandId;
      if (requestId) {
        const existing = existingBlocks.find(
          (block): block is ApprovalCardContentBlock =>
            block.type === "approval_card" &&
            block.approval_request_id === requestId,
        );
        blocks.push(
          existing
            ? { ...existing, state_version: projection.stateVersion }
            : this.approvalCardBlock(
                payload,
                projection.stateVersion,
                `current-approval:${requestId}`,
              ),
        );
      }
    } else if (
      projection.latestApprovalResolution?.["outcome"] === "accepted"
    ) {
      const requestId = protoString(
        projection.latestApprovalResolution["requestId"],
      );
      const approved = projection.latestApprovalResolution["approved"];
      const existing = existingBlocks.find(
        (block): block is ApprovalCardContentBlock =>
          block.type === "approval_card" &&
          block.approval_request_id === requestId,
      );
      if (existing && typeof approved === "boolean") {
        blocks.push({
          ...existing,
          decision: approved ? "approved" : "denied",
          decision_channel: "web",
          decision_submission: null,
          state_version: projection.stateVersion,
        });
      }
    }
    for (const summary of projection.pendingActions) {
      const requestPayload = summary["request"];
      const parsed = assistantActionRequestSchema.safeParse(requestPayload);
      const request = parsed.success
        ? parsed.data
        : recoverUnsupportedAssistantActionRequest(requestPayload);
      if (!request || request.actorId !== projection.actorId) continue;
      const resolved = resolveAssistantAction(request);
      const reports = Array.isArray(summary["reports"])
        ? summary["reports"]
        : [];
      const disposition = reports
        .slice()
        .reverse()
        .map((report) =>
          report && typeof report === "object" && !Array.isArray(report)
            ? (report as Record<string, unknown>)["disposition"]
            : undefined,
        )
        .find((value): value is string => typeof value === "string");
      const status: ActionCardStatus =
        summary["conflicted"] === true
          ? "conflicted"
          : disposition === "completed"
            ? "completed"
            : disposition === "declined"
              ? "declined"
              : disposition === "failed" ||
                  disposition === "cancelled" ||
                  disposition === "expired"
                ? "failed"
                : resolved.supported
                  ? "pending"
                  : "unsupported";
      blocks.push({
        type: "action_card",
        block_id: `current-action:${request.actionRequestId}`,
        action: request.action,
        action_request_id: request.actionRequestId,
        origin_turn_id: request.originTurnId,
        actor_id: request.actorId || undefined,
        task_id: request.taskId,
        step_id: request.stepId,
        params: resolved.params,
        status,
        outcome_note:
          status === "completed"
            ? "Reported — awaiting assistant verification."
            : "",
      });
      stored.actionRequestFingerprints.set(
        request.actionRequestId,
        fingerprintStableRequestInput(request.params),
      );
    }

    const managedTypes = new Set<ContentBlock["type"]>([
      "task_plan",
      "input_card",
      "approval_card",
      "action_card",
    ]);
    const stateMessageId = `assistant-current-state:${projection.actorId ?? stored.conversation.id}`;
    const retainedMessages = stored.turnState.messages.flatMap((message) => {
      if (message.id === stateMessageId) return [];
      const retainedBlocks = message.blocks.filter(
        (block) => !managedTypes.has(block.type),
      );
      if (retainedBlocks.length === 0 && message.blocks.length > 0) return [];
      return retainedBlocks.length === message.blocks.length
        ? [message]
        : [{ ...message, blocks: retainedBlocks }];
    });
    stored.turnState = {
      ...stored.turnState,
      messages:
        blocks.length === 0
          ? retainedMessages
          : [
              ...retainedMessages,
              {
                id: stateMessageId,
                role: "assistant",
                schema_version: 1,
                blocks,
                created_at:
                  projection.task?.updatedAt ??
                  projection.task?.createdAt ??
                  new Date(0).toISOString(),
                turnId:
                  projection.task?.turnId ??
                  protoString(projection.activeTurn?.["turnId"]),
              },
            ],
    };
  }

  private inputCardBlock(
    request: AssistantInputRequest,
    stateVersion?: number,
    blockId = newId("input-card"),
  ): InputCardContentBlock {
    return {
      type: "input_card",
      block_id: blockId,
      request_id: request.requestId,
      prompt: redactDisplayText(request.prompt),
      options: request.options.map((option) => ({
        option_id: option.optionId,
        label: redactDisplayText(option.label),
        ...(option.description
          ? { description: redactDisplayText(option.description) }
          : {}),
      })),
      allow_free_text: request.allowFreeText,
      multi_select: request.multiSelect,
      ...(stateVersion !== undefined ? { state_version: stateVersion } : {}),
      status: "pending",
    };
  }

  private approvalCardBlock(
    payload: ToolApprovalPayload,
    stateVersion?: number,
    blockId = newId("approval-card"),
  ): ApprovalCardContentBlock {
    const requestId =
      payload.requestId ?? payload.approvalRequestId ?? payload.commandId ?? "";
    const presentation = payload.presentation;
    const presentationBody = [
      presentation?.actorLabel,
      presentation?.action,
      presentation?.target ? `on ${presentation.target}` : "",
    ]
      .filter(Boolean)
      .join(" ");
    const body =
      (payload.message ?? payload.body ?? presentationBody) ||
      (payload.toolName
        ? `The assistant wants to run ${redactDisplayText(payload.toolName)}.`
        : "The assistant is requesting your approval to continue.");
    return {
      type: "approval_card",
      block_id: blockId,
      approval_request_id: requestId,
      body: redactDisplayText(body),
      service_slug: payload.serviceSlug ?? payload.service_slug ?? "",
      agent_key_prefix: payload.agentKeyPrefix ?? "aevatar",
      approval_mode: payload.approvalMode === "grant" ? "grant" : "per_request",
      grant_duration_sec:
        typeof payload.grantDurationSec === "number"
          ? payload.grantDurationSec
          : null,
      expires_at: payload.expiresAt ?? payload.expires_at ?? "",
      decision: null,
      decision_channel: null,
      decision_submission: null,
      ...(stateVersion !== undefined ? { state_version: stateVersion } : {}),
    };
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
      taskProjection: existing?.taskProjection,
      lastLocalTurnCompletedAt: existing?.lastLocalTurnCompletedAt,
      stateVersion:
        freshStateVersion === undefined
          ? existing?.stateVersion
          : Math.max(existing?.stateVersion ?? 0, freshStateVersion),
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
    stored.projectionPending = false;
    stored.projectionStalledAt = undefined;
    stored.requiredAssistantBaselineIds = undefined;
  }

  private newRun(
    onEvent: (event: TurnEvent) => void,
    turnId: string | null = null,
    protocol: "actor" = "actor",
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
      pendingApprovalRequestId: null,
      pendingInputRequestId: null,
      awaitingSignal: false,
      watchdog: null,
      deliveryStarted: false,
      deliveryTerminal: null,
      deliveryTerminalCount: 0,
      deliveryProtocolError: null,
      actionContinuation: null,
      optimisticMessageAppended: false,
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
        if (TYPED_SERVER_CONVERSATION_ID_PATTERN.test(stored.conversation.id)) {
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

  private async streamTurn(
    conversationId: string,
    run: RunningTurn,
    prompt: string,
  ): Promise<void> {
    // Serialize behind a previous turn's in-flight stop so this send cannot
    // arrive upstream before the fence commits.
    await this.awaitPendingStop(conversationId);

    let finalFailure = {
      code: "network_error",
      message: "The assistant stream could not be reached. Try again.",
    };

    const stored = this.conversations.get(conversationId);
    const isFirstTurn =
      stored?.conversation.id.startsWith(AEVATAR_DRAFT_CONVERSATION_PREFIX) ===
      true;
    const target = {
      url: TYPED_CHAT_URL,
      bodyText: JSON.stringify({
        // Aevatar dispatches `/api/chat` on this discriminator; the comparison
        // is ordinal, so the exact lowercase value matters.
        type: "text",
        ...(isFirstTurn ? {} : { conversationId: stored?.conversation.id }),
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
    if (run.finished || run.waitingForApproval || run.awaitingSignal) return;
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
      run.deliveryProtocolError = {
        code: "stream_protocol_error",
        message: "The assistant stream sent data after its terminal frame.",
      };
      return;
    }

    if (type !== "RUN_STARTED" && !isKeepalive && !run.deliveryStarted) {
      run.deliveryProtocolError ??= {
        code: "stream_protocol_error",
        message: "The assistant stream sent data before identifying the turn.",
      };
      return;
    }

    switch (type) {
      case "RUN_STARTED": {
        const authoritativeActorId =
          typeof frame.actorId === "string" &&
          TYPED_SERVER_CONVERSATION_ID_PATTERN.test(frame.actorId)
            ? frame.actorId
            : null;
        const authoritativeTurnId = safeTurnId(frame.turnId);
        const nestedThreadId = frame.runStarted?.threadId;
        const nestedRunId = frame.runStarted?.runId;
        const identityConflict =
          !authoritativeActorId ||
          !authoritativeTurnId ||
          (nestedThreadId !== undefined &&
            nestedThreadId !== authoritativeActorId) ||
          (nestedRunId !== undefined && nestedRunId !== authoritativeTurnId);
        if (run.deliveryStarted || identityConflict) {
          run.deliveryProtocolError ??= {
            code: "stream_protocol_error",
            message: run.deliveryStarted
              ? "The assistant stream started the same delivery more than once."
              : "The assistant stream returned missing or conflicting actor and turn identities.",
          };
          return;
        }
        const stored = this.conversations.get(conversationId);
        const currentActorId = stored?.conversation.id;
        if (
          !stored ||
          (currentActorId !== authoritativeActorId &&
            !currentActorId?.startsWith(AEVATAR_DRAFT_CONVERSATION_PREFIX))
        ) {
          run.deliveryProtocolError = {
            code: "stream_protocol_error",
            message:
              "The assistant stream belongs to a different conversation actor.",
          };
          return;
        }
        if (currentActorId !== authoritativeActorId) {
          stored.conversation = {
            ...stored.conversation,
            id: authoritativeActorId,
          };
          this.conversations.set(authoritativeActorId, stored);
          this.conversationAliases.set(conversationId, authoritativeActorId);
          if (this.activeConversationId === conversationId) {
            this.activeConversationId = authoritativeActorId;
          }
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
        const name = frame.stepStarted?.stepName ?? "actor-step";
        this.startRunStep(conversationId, run, name, name);
        return;
      }
      case "STEP_FINISHED": {
        const step = frame.stepFinished ?? {};
        const name = step.stepName ?? "actor-step";
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
        // Generic AG-UI snapshot telemetry is not the typed actor authority.
        return;
      case "CUSTOM": {
        this.handleCustomFrame(
          conversationId,
          run,
          frame.custom ?? {},
          frame.sequence,
        );
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
    sequence?: string | number,
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
            fingerprintActionRequest(request.data),
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
      case "nyxid.task.snapshot":
      case "nyxid.task.step.changed":
        this.applyLiveTaskFrame(conversationId, run, name, payload, sequence);
        return;
      case "nyxid.input.request": {
        const request = assistantInputRequestSchema.safeParse(payload);
        if (request.success) {
          this.addInputCard(conversationId, run, request.data);
        }
        return;
      }
      case "nyxid.input.changed":
        this.applyInputChanged(conversationId, run, payload);
        return;
      case "nyxid.approval.request": {
        if (payload && typeof payload === "object") {
          this.addApprovalCard(
            conversationId,
            run,
            payload as ToolApprovalPayload,
          );
        }
        return;
      }
      case "nyxid.approval.changed":
        this.applyApprovalChanged(conversationId, run, payload);
        return;
      case "aevatar.tool_approval.pending":
        this.addApprovalCard(
          conversationId,
          run,
          payload as ToolApprovalPayload,
        );
        return;
      case "aevatar.run.context":
      case "demo.conversation.context":
        // Correlation ids only; nothing renders.
        return;
      default:
        return;
    }
  }

  private applyLiveTaskFrame(
    conversationId: string,
    run: RunningTurn,
    name: "nyxid.task.snapshot" | "nyxid.task.step.changed",
    payload: unknown,
    sequence: unknown,
  ): void {
    const stored = this.conversations.get(conversationId);
    if (!stored) return;
    const projection = reduceTaskFrame(
      stored.taskProjection ?? createTaskProjection(conversationId),
      name,
      payload,
      sequence,
    );
    if (projection === stored.taskProjection || !projection.task) return;
    stored.taskProjection = projection;
    const existing = stored.turnState.messages
      .flatMap((message) => message.blocks)
      .find(
        (block): block is TaskPlanContentBlock =>
          block.type === "task_plan" &&
          block.plan.taskId === projection.task?.taskId,
      );
    if (existing) {
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.updated",
        block_id: existing.block_id,
        patch: {
          state_version: projection.stateVersion,
          progress_sequence: projection.progressSequence,
          plan: projection.task,
        },
      });
      return;
    }
    this.appendActivityBlock(conversationId, run, {
      type: "task_plan",
      block_id: `live-task-plan:${projection.task.taskId}`,
      state_version: projection.stateVersion,
      progress_sequence: projection.progressSequence,
      plan: projection.task,
    });
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
    // A blocked capability can be reported in the tool result even when the
    // optional status flags are absent, so inspect both sources.
    const blocker = parseToolResultBlocker(payload.result);
    if (blocker) this.addConnectCard(conversationId, run, blocker);
    const succeeded =
      !blocker && payload.success !== false && !/(ERROR|DENIED)/.test(status);
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
      blocker
        ? blocker.safeMessage
        : succeeded
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
    run.pendingApprovalRequestId = requestId;
    // A human gate has no client-imposed deadline; stop the watchdog.
    this.clearWatchdog(run);

    const block = this.approvalCardBlock(payload);
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

  private addInputCard(
    conversationId: string,
    run: RunningTurn,
    request: AssistantInputRequest,
  ): void {
    const dedupeKey = `input:${request.requestId}`;
    if (run.promptedApprovalIds.has(dedupeKey)) return;
    run.promptedApprovalIds.add(dedupeKey);
    run.awaitingSignal = true;
    run.pendingInputRequestId = request.requestId;
    this.clearWatchdog(run);

    const block = this.inputCardBlock(request);
    this.appendActivityBlock(conversationId, run, block);
    run.openCards.set(block.block_id, "input");
    this.markStepWaiting(
      conversationId,
      run,
      protoString(request["stepId"]),
      request.requestId,
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

  private applyInputChanged(
    conversationId: string,
    run: RunningTurn,
    payload: unknown,
  ): void {
    if (!payload || typeof payload !== "object") return;
    const requestId = protoString(
      (payload as Record<string, unknown>)["requestId"],
    );
    if (!requestId) return;
    const card = this.conversations
      .get(conversationId)
      ?.turnState.messages.flatMap((message) => message.blocks)
      .find(
        (block): block is InputCardContentBlock =>
          block.type === "input_card" && block.request_id === requestId,
      );
    if (!card) return;
    this.emitLocalBlockPatch(
      conversationId,
      card.block_id,
      { status: "resolved" },
      run.onEvent,
    );
    if (run.pendingInputRequestId === requestId) {
      run.pendingInputRequestId = null;
      run.awaitingSignal = false;
    }
  }

  private applyApprovalChanged(
    conversationId: string,
    run: RunningTurn,
    payload: unknown,
  ): void {
    if (!payload || typeof payload !== "object") return;
    const record = payload as Record<string, unknown>;
    const requestId = protoString(record["requestId"]);
    if (!requestId || typeof record["approved"] !== "boolean") return;
    const card = this.conversations
      .get(conversationId)
      ?.turnState.messages.flatMap((message) => message.blocks)
      .find(
        (block): block is ApprovalCardContentBlock =>
          block.type === "approval_card" &&
          block.approval_request_id === requestId,
      );
    if (!card) return;
    this.emitLocalBlockPatch(
      conversationId,
      card.block_id,
      {
        decision: record["approved"] ? "approved" : "denied",
        decision_channel: "web",
        decision_submission: null,
      },
      run.onEvent,
    );
    if (run.pendingApprovalRequestId === requestId) {
      run.pendingApprovalRequestId = null;
      run.waitingForApproval = false;
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
    const downloadUrl = safeMediaUrl(payload.url ?? "");
    if (downloadUrl) {
      this.appendActivityBlock(conversationId, run, {
        type: "artifact",
        block_id: newId("artifact"),
        artifact_id: newId("media"),
        name,
        mime,
        size_bytes: 0,
        preview: null,
        download_url: downloadUrl,
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
          if (!run.assistantContentObserved) {
            run.serverAnswerExpectation = "none";
          }
          this.finishTurn(conversationId, run, "blocked", null);
        } else {
          this.finalizeActivity(
            conversationId,
            run,
            run.waitingForApproval ? "waiting" : "done",
          );
          if (!run.assistantContentObserved) {
            run.serverAnswerExpectation = "none";
          }
          this.finishTurn(conversationId, run, "completed", null);
        }
    }
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

  private reserveHumanDecision(
    conversationId: string,
    expectedKind: "approval" | "input",
    blockId: string,
    requestId: string,
    onEvent: (event: TurnEvent) => void,
  ): RunningTurn {
    this.pauseAtHumanGate(conversationId, expectedKind, blockId, requestId);
    if (this.running.has(conversationId)) {
      throw new AssistantTurnActiveError();
    }
    const stored = this.conversations.get(conversationId);
    if (!stored) throw new AssistantConversationNotFoundError();
    const run = this.newRun(onEvent, null, "actor");
    run.cursor = stored.turnState.lastCursor;
    this.running.set(conversationId, run);
    return run;
  }

  private releaseHumanDecision(conversationId: string, run: RunningTurn): void {
    if (this.running.get(conversationId) !== run) return;
    run.finished = true;
    this.clearWatchdog(run);
    this.running.delete(conversationId);
  }

  private throwIfControlCancelled(run: RunningTurn): void {
    if (run.finished || run.controller.signal.aborted) {
      throw new AssistantTurnCancelledError();
    }
  }

  private async readDecisionPreflight(
    conversationId: string,
    kind: "approval" | "input",
    requestId: string,
    signal: AbortSignal,
  ): Promise<{
    readonly stateVersion: number;
    readonly committed: boolean;
    readonly approved?: boolean;
  }> {
    const current = await this.readCurrentDecisionState(conversationId, signal);
    const latest =
      current.snapshot[
        kind === "input" ? "latestInputResolution" : "latestApprovalResolution"
      ];
    if (latest && typeof latest === "object" && !Array.isArray(latest)) {
      const latestRecord = latest as Record<string, unknown>;
      if (
        protoString(latestRecord["requestId"]) === requestId &&
        latestRecord["outcome"] === "accepted"
      ) {
        if (kind === "input") {
          return { stateVersion: current.stateVersion, committed: true };
        }
        if (typeof latestRecord["approved"] === "boolean") {
          return {
            stateVersion: current.stateVersion,
            committed: true,
            approved: latestRecord["approved"],
          };
        }
      }
    }
    if (current.snapshot["attentionKind"] !== kind) {
      throw new AssistantProtocolError(
        `The assistant is no longer waiting for ${kind}.`,
      );
    }
    const pending =
      current.snapshot[kind === "input" ? "pendingInput" : "pendingApproval"];
    if (!pending || typeof pending !== "object" || Array.isArray(pending)) {
      throw new AssistantProtocolError(
        `The pending ${kind} request is no longer current.`,
      );
    }
    const pendingRecord = pending as Record<string, unknown>;
    const observedRequestId = protoString(
      kind === "input"
        ? pendingRecord["requestId"]
        : (pendingRecord["approvalRequestId"] ?? pendingRecord["requestId"]),
    );
    if (observedRequestId !== requestId) {
      throw new AssistantProtocolError(
        `The pending ${kind} request changed before it could be resolved.`,
      );
    }
    return { stateVersion: current.stateVersion, committed: false };
  }

  private applyCommittedApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
    stateVersion: number,
    onEvent: (event: TurnEvent) => void,
  ): void {
    this.emitLocalBlockPatch(
      conversationId,
      blockId,
      {
        decision: approved ? "approved" : "denied",
        decision_channel: "web",
        decision_submission: null,
        state_version: stateVersion,
      },
      onEvent,
    );
  }

  private async readCurrentDecisionState(
    conversationId: string,
    signal: AbortSignal,
  ): Promise<{
    readonly stateVersion: number;
    readonly snapshot: Record<string, unknown>;
  }> {
    const response = await assistantApi.get<unknown>(
      `${ASSISTANT_PREFIX}/conversations/${encodeURIComponent(conversationId)}/state`,
      signal,
    );
    if (!response || typeof response !== "object" || Array.isArray(response)) {
      throw new AssistantProtocolError(
        "The assistant state response was not a valid current-state envelope.",
      );
    }
    const envelope = response as Record<string, unknown>;
    if (envelope["status"] !== "current") {
      throw new AssistantProtocolError(
        "The assistant state is not current. Refresh the conversation and try again.",
      );
    }
    const stateVersion = positiveStateVersion(envelope["stateVersion"]);
    const snapshot = envelope["snapshot"];
    if (
      stateVersion === undefined ||
      !snapshot ||
      typeof snapshot !== "object" ||
      Array.isArray(snapshot)
    ) {
      throw new AssistantProtocolError(
        "The assistant state did not include an authoritative positive version.",
      );
    }
    const snapshotRecord = snapshot as Record<string, unknown>;
    if (protoString(snapshotRecord["actorId"]) !== conversationId) {
      throw new AssistantProtocolError(
        "The assistant state snapshot belongs to a different conversation.",
      );
    }
    if (positiveStateVersion(snapshotRecord["stateVersion"]) !== stateVersion) {
      throw new AssistantProtocolError(
        "The assistant state envelope and snapshot versions did not match.",
      );
    }
    return { stateVersion, snapshot: snapshotRecord };
  }

  private async observeDecisionCommit(
    conversationId: string,
    kind: "approval" | "input",
    requestId: string,
    expectedStateVersion: number,
    approved: boolean | undefined,
    signal: AbortSignal,
  ): Promise<number | null> {
    for (const delayMs of DECISION_OBSERVATION_DELAYS_MS) {
      await abortableDelay(delayMs, signal);
      const current = await this.readCurrentDecisionState(
        conversationId,
        signal,
      );
      const latest =
        current.snapshot[
          kind === "input"
            ? "latestInputResolution"
            : "latestApprovalResolution"
        ];
      if (latest && typeof latest === "object" && !Array.isArray(latest)) {
        const latestRecord = latest as Record<string, unknown>;
        const matchesDecision =
          protoString(latestRecord["requestId"]) === requestId &&
          latestRecord["outcome"] === "accepted" &&
          (kind === "input" || latestRecord["approved"] === approved);
        if (matchesDecision && current.stateVersion > expectedStateVersion) {
          return current.stateVersion;
        }
      }
      const pending =
        current.snapshot[kind === "input" ? "pendingInput" : "pendingApproval"];
      if (pending && typeof pending === "object" && !Array.isArray(pending)) {
        const pendingRecord = pending as Record<string, unknown>;
        const pendingRequestId = protoString(
          kind === "input"
            ? pendingRecord["requestId"]
            : (pendingRecord["approvalRequestId"] ??
                pendingRecord["requestId"]),
        );
        if (pendingRequestId !== requestId) {
          throw new AssistantProtocolError(
            `A different pending ${kind} request replaced the submitted request.`,
          );
        }
      }
    }
    return null;
  }

  /** Quietly settle a stream that is already parked at a human gate. */
  private pauseAtHumanGate(
    conversationId: string,
    expectedKind: "approval" | "input",
    blockId: string,
    requestId: string,
  ): void {
    const run = this.running.get(conversationId);
    if (!run) return;
    const exactRequestId =
      expectedKind === "approval"
        ? run.pendingApprovalRequestId
        : run.pendingInputRequestId;
    if (
      exactRequestId !== requestId ||
      run.openCards.get(blockId) !== expectedKind
    ) {
      throw new AssistantTurnActiveError();
    }
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
    if (run.turnId) {
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
   * idempotent upstream. The transport first reads actor-owned current state
   * and sends that exact version; Stop never bypasses the stale-state fence.
   * Requires the server-announced `turnId` (a
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
    const pending = this.dispatchTaskStop(
      actorConversationId,
      crypto.randomUUID(),
      deadline.signal,
      run.turnId,
    ).then(
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
