export type ChannelPlatform =
  | "telegram"
  | "discord"
  | "lark"
  | "feishu"
  | "slack";

/**
 * All platform values a conversation may report. `"device"` is for HTTP
 * Event Gateway channels (NyxID#221) and, unlike the bot platforms, is
 * never a valid `ChannelBot.platform`.
 */
export type ConversationPlatform = ChannelPlatform | "device";

export type ChannelBotStatus =
  | "pending"
  | "pending_webhook"
  | "active"
  | "failed"
  | "invalid";

export type ConversationType = "private" | "group" | "channel" | "device";

export type MessageDirection = "inbound" | "outbound";

export type CallbackStatus = "pending" | "delivered" | "failed" | "timeout";

export type ContentType =
  | "text"
  | "image"
  | "file"
  | "audio"
  | "video"
  | "location"
  | "sticker"
  | "unknown";

export interface ChannelBotItem {
  readonly id: string;
  readonly platform: ChannelPlatform;
  readonly label: string;
  readonly platform_bot_id: string;
  readonly platform_bot_username: string;
  readonly webhook_registered: boolean;
  readonly status: ChannelBotStatus;
  readonly is_active: boolean;
  readonly created_at: string;
  readonly updated_at: string;
  /** Effective owner user_id. For personal bots this equals the caller's
   *  user id; for org-owned bots it equals the org's user id (which is
   *  also the value clients pass as `target_org_id`). */
  readonly user_id: string;
}

export interface ChannelBotListResponse {
  readonly bots: readonly ChannelBotItem[];
  readonly total: number;
}

export interface ChannelBotDetail extends ChannelBotItem {
  readonly conversations_count: number;
  readonly app_secret_configured: boolean;
  readonly lark_verification_token_configured: boolean;
  readonly lark_encrypt_key_configured: boolean;
  /** Lark/Feishu only: deep link to the developer console permissions
   *  page with the scopes NyxID's adapter needs already pre-selected.
   *  `null` for non-Lark platforms or when the bot has no `app_id`. */
  readonly permission_setup_url?: string | null;
  /** Lark/Feishu only: scope keys encoded in `permission_setup_url`,
   *  echoed back so the UI can render the list under the link. */
  readonly permission_setup_scopes?: readonly string[] | null;
}

export interface CreateChannelBotRequest {
  readonly platform: ChannelPlatform;
  readonly bot_token: string;
  readonly label: string;
  /** Lark/Feishu only */
  readonly app_id?: string;
  /** Lark/Feishu: app secret. Slack: app signing secret. */
  readonly app_secret?: string;
  /** Lark/Feishu only */
  readonly verification_token?: string;
  /** Lark/Feishu only */
  readonly encrypt_key?: string;
  /** Discord only */
  readonly public_key?: string;
  /** Create this bot under the given org (caller must be admin). */
  readonly target_org_id?: string;
}

export interface UpdateChannelBotRequest {
  readonly label?: string;
  readonly verification_token?: string;
  readonly encrypt_key?: string;
  readonly app_id?: string;
  readonly app_secret?: string;
}

export interface CreateChannelBotResponse {
  readonly id: string;
  readonly platform: ChannelPlatform;
  readonly platform_bot_username: string;
  readonly status: ChannelBotStatus;
  readonly permission_setup_url?: string | null;
  readonly permission_setup_scopes?: readonly string[] | null;
}

export interface ChannelConversationItem {
  readonly id: string;
  /** `null` or omitted for device channels (platform === "device"). */
  readonly channel_bot_id: string | null;
  readonly platform: ConversationPlatform;
  readonly platform_conversation_id: string;
  readonly platform_conversation_type: ConversationType;
  readonly platform_sender_id: string | null;
  readonly agent_api_key_id: string;
  readonly default_agent: boolean;
  readonly is_active: boolean;
  readonly last_message_at: string | null;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface ChannelConversationListResponse {
  readonly conversations: readonly ChannelConversationItem[];
  readonly total: number;
}

export interface CreateChannelConversationRequest {
  readonly channel_bot_id: string;
  readonly agent_api_key_id: string;
  readonly platform_conversation_id?: string;
  readonly platform_conversation_type?: ConversationType;
  readonly platform_sender_id?: string;
  readonly default_agent?: boolean;
  /** Create this conversation under the given org (caller must be admin). */
  readonly target_org_id?: string;
}

/**
 * Request body for creating a device channel (HTTP Event Gateway, NyxID#221).
 * No backing bot is required or allowed; the conversation is identified
 * directly by `platform_conversation_id`.
 */
export interface CreateDeviceConversationRequest {
  readonly platform: "device";
  readonly platform_conversation_id: string;
  readonly agent_api_key_id: string;
  readonly platform_conversation_type?: string;
  readonly target_org_id?: string;
}

export interface UpdateChannelConversationRequest {
  readonly agent_api_key_id?: string;
  readonly default_agent?: boolean;
  readonly is_active?: boolean;
}

/**
 * Metadata-only message summary returned by the backend's
 * `GET /channel-conversations/{id}/messages` and `/channel-relay/messages/{id}`
 * endpoints.
 *
 * **Per ADR-013 (NyxID Pure Passthrough), message content is not stored.**
 * The `text` and `attachments` fields that used to live here were removed —
 * the message body lives with the downstream agent (e.g. Aevatar grain state)
 * and NyxID retains only routing metadata.
 */
export interface ChannelMessageItem {
  readonly id: string;
  /** `null` for messages on device channels. */
  readonly channel_bot_id: string | null;
  readonly conversation_id: string;
  readonly direction: MessageDirection;
  readonly platform: ConversationPlatform;
  readonly platform_message_id: string | null;
  readonly sender_platform_id: string | null;
  readonly sender_display_name: string | null;
  readonly content_type: ContentType;
  readonly agent_api_key_id: string | null;
  readonly callback_status: CallbackStatus | null;
  readonly reply_to_message_id: string | null;
  readonly created_at: string;
}

export interface ChannelMessageListResponse {
  readonly messages: readonly ChannelMessageItem[];
  readonly total: number;
  readonly page: number;
  readonly per_page: number;
}

export interface ChannelRelayReplyRequest {
  readonly message_id: string;
  readonly reply: {
    readonly text?: string;
    readonly metadata?: Record<string, unknown>;
  };
}
