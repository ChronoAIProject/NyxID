import type { ActionCardParams } from "@/schemas/assistant-actions";

export type ConnectCardState =
  | "needs_connection"
  | "waiting_for_provider"
  | "waiting_for_user"
  | "connected"
  | "error"
  | "timed_out";

export type ApprovalDecision = "approved" | "denied" | "expired" | "cancelled";

export type ApprovalDecisionChannel = "web" | "telegram" | "mobile";

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
  readonly reason_code?: "NYXID_SERVICE_NOT_CONNECTED" | "NYXID_UNAUTHORIZED";
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
  readonly decision_submission?: "approved" | "denied" | null;
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
  readonly actor_id?: string;
  readonly task_id: string;
  readonly step_id: string;
  readonly params: ActionCardParams;
  readonly status: ActionCardStatus;
  readonly outcome_note: string;
}

export interface Conversation {
  readonly id: string;
  readonly title: string;
  readonly created_at: string;
  readonly last_message_at: string;
  readonly message_count?: number;
  readonly llm_route?: string | null;
  readonly llm_model?: string | null;
}
