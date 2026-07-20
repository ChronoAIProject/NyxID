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
    readonly service_slug: string | null;
    readonly artifact_id: string | null;
    readonly approval_request_id: string | null;
  }>;
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

export type ContentBlock =
  | TextContentBlock
  | ConnectCardContentBlock
  | RunContentBlock
  | ApprovalCardContentBlock
  | ArtifactContentBlock;

export interface AssistantMessage {
  readonly id: string;
  readonly role: "user" | "assistant" | "system";
  readonly schema_version: number;
  readonly blocks: ContentBlock[];
  readonly created_at: string;
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
}

export type TurnStatus =
  | "running"
  | "waiting"
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
      readonly turn_id: string;
      readonly status: "completed" | "failed" | "cancelled";
      readonly error: {
        readonly code: string;
        readonly message: string;
      } | null;
    });

export interface ActiveTurn {
  readonly turnId: string;
  readonly status: TurnStatus;
  readonly error: { readonly code: string; readonly message: string } | null;
}

export interface TurnReducerState {
  readonly messages: AssistantMessage[];
  readonly activeTurn: ActiveTurn | null;
  readonly lastCursor: number;
}

export interface TurnHandle {
  readonly turnId: string;
  cancel(): void;
}

export interface AssistantTransport {
  listConversations(): Promise<Conversation[]>;
  createConversation(): Promise<Conversation>;
  getHistory(conversationId: string): Promise<ConversationHistory>;
  deleteConversation(conversationId: string): Promise<void>;
  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle;
  /**
   * Send an approval decision. On the live Aevatar contract the approve
   * endpoint responds with an SSE continuation of the run; implementations
   * that stream it return a cancellable handle for the continuation turn and
   * deliver its events to `onEvent`. Implementations with no continuation
   * (mock) return null.
   */
  decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
    onEvent?: (event: TurnEvent) => void,
  ): Promise<TurnHandle | null>;
}

export function isTurnActive(status: TurnStatus | undefined): boolean {
  return status === "running" || status === "waiting";
}
