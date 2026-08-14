import type {
  ActionCardParams,
  ActionReport,
} from "@/schemas/assistant-actions";
import type { InputAnswer } from "@/schemas/assistant-input";

export type ConnectCardState =
  | "needs_connection"
  | "waiting_for_provider"
  | "waiting_for_user"
  | "connected"
  | "error"
  | "timed_out";

export type RunState =
  | "running"
  | "awaiting_approval"
  | "awaiting_connection"
  | "completed"
  | "failed"
  | "cancelled";

export type RunStepStatus =
  | "done"
  | "active"
  | "waiting"
  | "failed"
  | "skipped";

export type TaskStatus =
  | "active"
  | "succeeded"
  | "failed"
  | "stopped"
  | "blocked";

export type TaskStepStatus =
  | "planned"
  | "waiting"
  | "running"
  | "done"
  | "failed"
  | "skipped"
  | "cancelled"
  | "uncertain";

export type TaskExternalEffect =
  | "not_started"
  | "not_applied"
  | "confirmed"
  | "may_have_changed";

export type TaskStepKind =
  | "llm"
  | "tool"
  | "browser_action"
  | "postcondition"
  | "input"
  | "approval"
  | "web"
  | "condition";

export interface TaskAvailableActions {
  readonly retry: boolean;
  readonly skip: boolean;
  readonly stop: boolean;
}

export interface TaskSubstep {
  readonly substepId: string;
  readonly title: string;
  readonly status: "running" | "done" | "failed";
}

export interface TaskNumericCondition {
  readonly conditionId: string;
  readonly sourceInputRequestId: string;
  readonly suggestedThreshold: number;
  readonly effectiveThreshold: number;
  readonly thresholdOrigin: "suggested" | "user_override";
  readonly observedValue: number;
  readonly comparison: "gte";
  readonly outcome: "true" | "false";
  readonly evaluatedAt?: string;
  readonly guardedToolName: string;
}

export type TaskStepSource =
  | { readonly kind: "llm"; readonly label: string }
  | {
      readonly kind: "tool";
      readonly label: string;
      readonly serviceSlug?: string;
      readonly serviceId?: string;
    }
  | { readonly kind: "browserAction"; readonly label: string }
  | { readonly kind: "postcondition"; readonly label: string }
  | { readonly kind: "input"; readonly label: string }
  | { readonly kind: "approval"; readonly label: string }
  | { readonly kind: "web"; readonly label: string }
  | {
      readonly kind: "condition";
      readonly label: string;
      readonly condition: TaskNumericCondition;
    };

export interface TaskOperation {
  readonly conversationActorId?: string;
  readonly turnId?: string;
  readonly taskId?: string;
  readonly stepId?: string;
  readonly operationId?: string;
  readonly operationGeneration?: number;
  readonly kind?: string;
  readonly phase?: string;
  readonly latestProgressSequence?: number;
  readonly safeMessage?: string;
  readonly failureCode?: string;
  readonly progressMessage?: string;
  readonly startedAt?: string;
  readonly updatedAt?: string;
  readonly completedAt?: string;
  readonly lastProgressAt?: string;
  readonly stalledAt?: string;
}

export interface TaskApprovalObservation {
  readonly approvalRequestId: string;
  readonly decisionMode: "unknown" | "per_request" | "grant";
  readonly receiptStatus: "approval_required" | "denied";
  readonly observedAt: string;
  readonly terminalOutcome?: "rejected" | "expired" | "timed_out";
  readonly subjectKind?: string;
}

export interface TaskStep {
  readonly stepId: string;
  readonly order: number;
  readonly kind: TaskStepKind;
  readonly status: TaskStepStatus;
  readonly required: boolean;
  readonly description: string;
  readonly source: TaskStepSource;
  readonly mayChangeExternalState: boolean;
  readonly externalEffect: TaskExternalEffect;
  readonly availableActions: TaskAvailableActions;
  readonly updatedAt?: string;
  readonly addedBy?:
    | "initial"
    | "scope_resolution"
    | "replan"
    | "steering"
    | "user_revision";
  readonly addedInPlanRevision?: number;
  readonly cancelledInPlanRevision?: number;
  readonly dependsOn: readonly string[];
  readonly estimate?: { readonly kind: "duration"; readonly seconds: number };
  readonly substeps: readonly TaskSubstep[];
  readonly operation?: TaskOperation | null;
  readonly approvalObservation?: TaskApprovalObservation;
  readonly guard?: {
    readonly conditionStepId: string;
    readonly requiredOutcome: "true" | "false";
  };
  readonly actionRequestId?: string;
  readonly approvalRequestId?: string;
  readonly failureCode?: string;
  readonly safeMessage?: string;
  readonly safeToSkip?: boolean;
}

export interface TaskPlanGate {
  readonly mode: "auto" | "confirm";
  readonly status?: "pending" | "satisfied" | "rejected";
  readonly requestId?: string;
  readonly taskId?: string;
  readonly planId?: string;
  readonly planRevision?: number;
  readonly reason?: string;
  readonly decidedAt?: string;
}

export interface TaskPlan {
  readonly schemaVersion: number;
  readonly actorId: string;
  readonly taskId: string;
  readonly turnId: string;
  readonly planId: string;
  readonly planRevision: number;
  readonly planRevisionHistoryStart?: number;
  readonly planRevisions: readonly {
    readonly planRevision: number;
    readonly revisionCause:
      | "initial"
      | "scope_resolution"
      | "failure_recovery"
      | "steering"
      | "user_revision";
    readonly committedAt?: string;
    readonly addedStepIds: readonly string[];
    readonly cancelledStepIds: readonly string[];
  }[];
  readonly title: string;
  readonly status: TaskStatus;
  readonly activeStepId?: string;
  readonly failureCode?: string;
  readonly safeMessage?: string;
  readonly createdAt?: string;
  readonly updatedAt?: string;
  readonly gate?: TaskPlanGate | null;
  readonly steps: readonly TaskStep[];
}

export type ApprovalDecision = "approved" | "denied" | "expired" | "cancelled";

export type ApprovalDecisionChannel = "web" | "telegram" | "mobile";

export interface TextContentBlock {
  readonly type: "text";
  readonly block_id: string;
  readonly text: string;
}

export interface ConnectCardContentBlock {
  readonly type: "connect_card";
  readonly block_id: string;
  readonly catalog_slug: string;
  readonly service_name: string;
  readonly icon_url: string;
  readonly subtitle: string;
  readonly auth_kind: "oauth" | "api_key" | "device_code";
  readonly requested_scopes: string[];
  readonly key_id: string | null;
  readonly granted_scopes: string[] | null;
  readonly device_user_code: string | null;
  readonly device_verification_url: string | null;
  readonly state: ConnectCardState;
  readonly error_message: string | null;
  readonly steps: Array<{
    readonly title: string;
    readonly body: string;
    readonly done: boolean;
  }>;
  readonly footer: string;
  /** Exact Aevatar authorization blocker classification, when provided. */
  readonly reason_code?: "NYXID_SERVICE_NOT_CONNECTED" | "NYXID_UNAUTHORIZED";
}

export interface RunContentBlock {
  readonly type: "run";
  readonly block_id: string;
  readonly title: string;
  readonly steps_total: number;
  readonly steps_complete: number;
  readonly state: RunState;
  readonly steps: Array<{
    readonly index: number;
    readonly status: RunStepStatus;
    readonly label: string;
    readonly meta: string;
    readonly result_preview?: string | null;
    readonly service_slug: string | null;
    readonly artifact_id: string | null;
    readonly approval_request_id: string | null;
  }>;
}

export interface TaskPlanContentBlock {
  readonly type: "task_plan";
  readonly block_id: string;
  readonly state_version: number;
  readonly progress_sequence: number;
  readonly plan: TaskPlan;
}

export interface ApprovalCardContentBlock {
  readonly type: "approval_card";
  readonly block_id: string;
  readonly approval_request_id: string;
  readonly body: string;
  readonly service_slug: string;
  readonly agent_key_prefix: string;
  readonly approval_mode: "per_request" | "grant";
  readonly grant_duration_sec: number | null;
  readonly expires_at: string;
  readonly decision: ApprovalDecision | null;
  readonly decision_channel: ApprovalDecisionChannel | null;
  /** 202 accepted, awaiting the matching committed resolution fact. */
  readonly decision_submission?: "approved" | "denied" | null;
  /** Authoritative current-state version, when rehydrated from `/state`. */
  readonly state_version?: number;
}

export type InputCardStatus =
  | "pending"
  | "submitted"
  | "resolved"
  | "cancelled";

export interface InputCardContentBlock {
  readonly type: "input_card";
  readonly block_id: string;
  readonly request_id: string;
  readonly prompt: string;
  readonly options: readonly {
    readonly option_id: string;
    readonly label: string;
    readonly description?: string;
  }[];
  readonly allow_free_text: boolean;
  readonly multi_select: boolean;
  readonly state_version?: number;
  readonly status: InputCardStatus;
}

export interface ArtifactContentBlock {
  readonly type: "artifact";
  readonly block_id: string;
  readonly artifact_id: string;
  readonly name: string;
  readonly mime: string;
  readonly size_bytes: number;
  readonly preview: string | null;
  readonly download_url: string;
}

export type ActionCardStatus =
  | "pending"
  | "in_progress"
  | "blocked"
  | "completed"
  | "conflicted"
  | "declined"
  | "failed"
  | "unsupported";

export interface ActionCardContentBlock {
  readonly type: "action_card";
  readonly block_id: string;
  readonly action: string;
  readonly action_request_id: string;
  readonly origin_turn_id: string;
  /** Actor that emitted the request; used only to address action.continue. */
  readonly actor_id?: string;
  readonly task_id: string;
  readonly step_id: string;
  readonly params: ActionCardParams;
  readonly status: ActionCardStatus;
  readonly outcome_note: string;
}

export type ContentBlock =
  | TextContentBlock
  | ConnectCardContentBlock
  | RunContentBlock
  | TaskPlanContentBlock
  | ApprovalCardContentBlock
  | InputCardContentBlock
  | ArtifactContentBlock
  | ActionCardContentBlock;

export interface AssistantMessage {
  readonly id: string;
  readonly role: "user" | "assistant" | "system";
  readonly schema_version: number;
  readonly blocks: ContentBlock[];
  readonly created_at: string;
  /** Server-owned turn metadata retained when hydrating Aevatar history. */
  readonly turnId?: string;
  readonly status?: TurnStatus;
  readonly error?:
    | string
    | {
        readonly code: string;
        readonly message: string;
      }
    | null;
}

export interface Conversation {
  readonly id: string;
  readonly title: string;
  readonly created_at: string;
  readonly last_message_at: string;
  /** Materialized message count from the Chat History index, when known. */
  readonly message_count?: number;
  /** LLM route/model recorded by the Chat History index, when known. */
  readonly llm_route?: string | null;
  readonly llm_model?: string | null;
}

export interface ConversationHistory {
  readonly conversation: Conversation;
  readonly messages: AssistantMessage[];
  readonly has_more: boolean;
  /** The local stream mirror is authoritative while identity/history catches up. */
  readonly awaitingProjection?: boolean;
  /** Background reconciliation reached its deadline without proving absence. */
  readonly projectionStalled?: boolean;
}

export interface ProjectionReconcileOutcome {
  readonly status: "materialized" | "absent" | "timed_out";
  readonly conversationId: string;
}

export type TurnStatus =
  | "running"
  | "waiting"
  | "blocked"
  | "completed"
  | "failed"
  | "cancelled";

interface TurnEventBase {
  readonly cursor: number;
}

export type TurnEvent =
  | (TurnEventBase & {
      readonly event: "turn.status";
      readonly turn_id: string;
      readonly status: TurnStatus;
    })
  | (TurnEventBase & {
      readonly event: "message.started";
      readonly message_id: string;
      readonly role: "assistant";
    })
  | (TurnEventBase & {
      readonly event: "block.started";
      readonly message_id: string;
      readonly block_id: string;
      readonly index: number;
      readonly block: ContentBlock;
    })
  | (TurnEventBase & {
      readonly event: "block.delta";
      readonly block_id: string;
      readonly text: string;
    })
  | (TurnEventBase & {
      readonly event: "block.updated";
      readonly block_id: string;
      readonly patch: Partial<ContentBlock>;
    })
  | (TurnEventBase & {
      readonly event: "block.completed";
      readonly block_id: string;
      readonly block: ContentBlock;
    })
  | (TurnEventBase & {
      readonly event: "message.completed";
      readonly message_id: string;
    })
  | (TurnEventBase & {
      readonly event: "turn.completed";
      readonly turn_id: string | null;
      readonly status: "blocked" | "completed" | "failed" | "cancelled";
      readonly error: {
        readonly code: string;
        readonly message: string;
      } | null;
    });

/**
 * Loading state for ONE stream episode, owned by the event pump that serves it.
 *
 * A send and an approval continuation each get their own pump, which makes the
 * pump the only thing in the client that knows where one episode ends and the
 * next begins. Neither the transcript nor the turn id can stand in for it: a
 * continuation is appended to the previous turn's assistant group AND reuses its
 * server turn id, so "has this episode said anything yet" is unanswerable from
 * either. That question is what the streaming dots and the empty-turn error
 * both turn on.
 */
export interface TurnEpisode {
  /** Pump created, no terminal event delivered yet. */
  readonly open: boolean;
  /** This episode has emitted at least one block a reader can see. */
  readonly printed: boolean;
  /** A transcript projection for this episode is in flight. */
  readonly projecting: boolean;
}

export interface ActiveTurn {
  readonly turnId: string | null;
  readonly status: TurnStatus;
  readonly error: { readonly code: string; readonly message: string } | null;
}

export interface TurnReducerState {
  readonly messages: AssistantMessage[];
  readonly activeTurn: ActiveTurn | null;
  readonly lastCursor: number;
}

export interface TurnHandle {
  /** Null until Aevatar publishes the authoritative `RUN_STARTED.turnId`. */
  readonly turnId: string | null;
  cancel(): void;
}

export interface AssistantTransport {
  listConversations(): Promise<Conversation[]>;
  createConversation(): Promise<Conversation>;
  getHistory(conversationId: string): Promise<ConversationHistory>;
  reconcileProjection(
    conversationId: string,
  ): Promise<ProjectionReconcileOutcome>;
  releaseProjectionWaiter(conversationId: string): void;
  deleteConversation(conversationId: string): Promise<void>;
  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle;
  /**
   * Cancel the conversation's live turn by id. Covers turns whose handle
   * the caller does not hold — e.g. an approval continuation still waiting
   * on response headers. No-op when nothing is running.
   */
  cancelActiveTurn(conversationId: string): void;
  /** Stop the actor-owned task against the latest committed state version. */
  stopTask(conversationId: string): Promise<void>;
  /** Steer active actor work without opening a competing text turn. */
  steerTask(conversationId: string, instruction: string): Promise<void>;
  retryStep(conversationId: string, stepId: string): Promise<void>;
  skipStep(conversationId: string, stepId: string): Promise<void>;
  /** Resolve the exact pending plan gate against a fresh actor-state fence. */
  resolvePlan(
    conversationId: string,
    blockId: string,
    confirmed: boolean,
  ): Promise<void>;
  /**
   * Send a version-fenced approval decision. The command returns JSON
   * acceptance; later committed projection frames carry business progress.
   */
  decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
    onEvent?: (event: TurnEvent) => void,
  ): Promise<TurnHandle | null>;
  resolveInput(
    conversationId: string,
    blockId: string,
    answer: InputAnswer,
    onEvent?: (event: TurnEvent) => void,
  ): Promise<TurnHandle | null>;
  setActionCardInProgress(
    conversationId: string,
    blockId: string,
    inProgress: boolean,
    onEvent?: (event: TurnEvent) => void,
  ): void;
  blockActionCard(
    conversationId: string,
    blockId: string,
    note: string,
    onEvent?: (event: TurnEvent) => void,
  ): void;
  continueActions(
    conversationId: string,
    originTurnId: string,
    reports: readonly ActionReport[],
    onEvent?: (event: TurnEvent) => void,
  ): TurnHandle | null;
  /** Resume a blocked typed turn without fabricating an action report. */
  wakeActions(
    conversationId: string,
    originTurnId: string,
    onEvent?: (event: TurnEvent) => void,
  ): TurnHandle;
}

export function isTurnActive(status: TurnStatus | undefined): boolean {
  return status === "running" || status === "waiting";
}
