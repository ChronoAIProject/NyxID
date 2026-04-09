export type ChannelPlatform = "telegram" | "discord" | "lark" | "feishu";

export type ChannelBotStatus = "pending" | "active" | "failed" | "invalid";

export type ConversationType = "private" | "group" | "channel";

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
}

export interface ChannelBotListResponse {
  readonly bots: readonly ChannelBotItem[];
  readonly total: number;
}

export interface ChannelBotDetail extends ChannelBotItem {
  readonly conversations_count: number;
}

export interface CreateChannelBotRequest {
  readonly platform: ChannelPlatform;
  readonly bot_token: string;
  readonly label: string;
  /** Lark/Feishu only */
  readonly app_id?: string;
  /** Lark/Feishu only */
  readonly app_secret?: string;
  /** Discord only */
  readonly public_key?: string;
}

export interface CreateChannelBotResponse {
  readonly id: string;
  readonly platform: ChannelPlatform;
  readonly platform_bot_username: string;
  readonly status: ChannelBotStatus;
}

export interface ChannelConversationItem {
  readonly id: string;
  readonly channel_bot_id: string;
  readonly platform: ChannelPlatform;
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
  readonly channel_bot_id: string;
  readonly conversation_id: string;
  readonly direction: MessageDirection;
  readonly platform: ChannelPlatform;
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
