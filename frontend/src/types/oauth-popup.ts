/** Mirrors backend/src/models/oauth_flow_kind.rs. */
export const OAUTH_FLOW_TOKENS = ["cc", "kc", "pc", "sa", "wz", "sl"] as const;
export type OAuthFlowKind = (typeof OAUTH_FLOW_TOKENS)[number];

export const OAUTH_ERROR_CODES = [
  "access_denied",
  "provider_error",
  "state_expired",
  "state_replayed",
  "state_invalid",
  "session_mismatch",
  "session_required",
  "exchange_failed",
  "server_error",
] as const;
/** `session_required` and `server_error` are reserved for future popup flows. */
export type OAuthErrorCode = (typeof OAUTH_ERROR_CODES)[number];
export type OAuthCompletionStatus = "complete" | "error";

export interface OAuthResultMessage {
  readonly type: "oauth_result";
  readonly status: OAuthCompletionStatus;
  readonly flow?: OAuthFlowKind;
  readonly code?: OAuthErrorCode;
}

export interface OAuthActionMessage {
  readonly type: "oauth_action";
  readonly action: "view_result" | "retry" | "dismiss";
}

export interface OAuthAckMessage {
  readonly type: "oauth_ack";
}

export interface OAuthRetryMessage {
  readonly type: "oauth_retry";
  readonly nextNonce: string;
  readonly url: string;
}

export type OAuthChannelMessage =
  | OAuthResultMessage
  | OAuthActionMessage
  | OAuthAckMessage
  | OAuthRetryMessage;

export interface OAuthLaunchReadyMessage {
  readonly type: "oauth_launch_ready";
  readonly launchId: string;
}

export interface OAuthLaunchNavigateMessage {
  readonly type: "oauth_launch_navigate";
  readonly launchId: string;
  readonly nonce: string;
  readonly url: string;
  readonly serviceName?: string;
}
