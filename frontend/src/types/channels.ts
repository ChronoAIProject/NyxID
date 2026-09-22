/** Platform identifiers are supplied by the server catalog. */
export type ChannelPlatform = string;

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
  | "invalid"
  | "suspended";

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
  readonly credential_source?: "user" | "platform" | "connection" | "telegram_manager";
  readonly managed_setup?: ManagedBotSetup | null;
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
  readonly last_verification?: BotVerification | null;
  readonly webhook_ingestion?: boolean;
  readonly connection_id?: string | null;
  readonly poll_cursor?: string | null;
  readonly last_polled_at?: string | null;
  readonly next_poll_at?: string | null;
  readonly poll_backoff_until?: string | null;
  readonly poll_error_count?: number;
  readonly last_poll_notice?: string | null;
  readonly error?: string | null;
  readonly phone_number_id?: string;
  readonly waba_id?: string;
  readonly webhook_url?: string;
  readonly webhook_secret_label?: string | null;
  readonly setup_instructions?: readonly string[];
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

export interface BotVerification {
  readonly id: string;
  readonly status: "pending" | "verified" | "failed" | "incomplete";
  readonly started_at: string;
  readonly completed_at: string | null;
  readonly message: string | null;
}

export interface VerifyChannelBotResponse {
  readonly id: string;
  readonly status: ChannelBotStatus;
  readonly webhook_registered: boolean;
  readonly last_verification?: BotVerification | null;
}

export interface CreateChannelBotRequest {
  readonly [field: string]: string | undefined;
  readonly platform: ChannelPlatform;
  readonly bot_token?: string;
  readonly label: string;
  /** Lark/Feishu only */
  readonly app_id?: string;
  /** Lark/Feishu: app secret. Slack: signing secret. WhatsApp: Meta App Secret. */
  readonly app_secret?: string;
  /** Lark/Feishu only */
  readonly verification_token?: string;
  /** Lark/Feishu only */
  readonly encrypt_key?: string;
  /** Discord only */
  readonly public_key?: string;
  readonly phone_number_id?: string;
  readonly waba_id?: string;
  /** Create this bot under the given org (caller must be admin). */
  readonly target_org_id?: string;
}

export interface UpdateChannelBotRequest {
  readonly [field: string]: string | undefined;
  readonly bot_token?: string;
  readonly label?: string;
  readonly verification_token?: string;
  readonly encrypt_key?: string;
  readonly app_id?: string;
  readonly app_secret?: string;
}

export interface CreateChannelBotResponse {
  readonly credential_source?: "user" | "platform" | "connection" | "telegram_manager";
  readonly webhook_ingestion?: boolean;
  readonly connection_id?: string | null;
  readonly managed_setup?: ManagedBotSetup | null;
  readonly phone_number_id?: string;
  readonly waba_id?: string;
  readonly webhook_url?: string;
  readonly webhook_secret?: string | null;
  readonly webhook_secret_label?: string | null;
  readonly setup_instructions?: readonly string[];
  readonly id: string;
  readonly platform: ChannelPlatform;
  readonly platform_bot_username: string;
  readonly status: ChannelBotStatus;
  readonly permission_setup_url?: string | null;
  readonly permission_setup_scopes?: readonly string[] | null;
}

export interface ManagedBotSetup {
  readonly subscription: string;
  readonly webhook_override: string;
  readonly registration: string;
  readonly business_id?: string | null;
  readonly coexistence: boolean;
  readonly coexistence_sync?: Record<string, string>;
}

export interface OutboundCapabilities {
  readonly media: MediaCapabilities;
  readonly initiated_send: boolean;
  readonly reply_to: boolean;
  readonly thread: boolean;
  readonly edit: boolean;
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
  readonly allow_agent_initiated: boolean;
  readonly capabilities: OutboundCapabilities;
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
  readonly allow_agent_initiated?: boolean;
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
  readonly allow_agent_initiated?: boolean;
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
export type ChannelDeliveryStatus = "accepted" | "sent" | "delivered" | "read" | "played" | "failed" | "partial" | "unknown" | "legacy_final_only" | "recipient_only";
export interface ChannelDeliveryComponent {
  readonly platform_message_id: string;
  readonly status: ChannelDeliveryStatus;
  readonly sent_at: string | null;
  readonly delivered_at: string | null;
  readonly read_at: string | null;
  readonly played_at: string | null;
  readonly failed_at: string | null;
  readonly error_codes: readonly number[];
}
export interface ChannelDelivery {
  readonly status: ChannelDeliveryStatus;
  /** All send components were accepted and their IDs are recorded. */
  readonly complete: boolean;
  readonly expected_components: number | null;
  readonly recipient_only: boolean;
  readonly failure_code: number | null;
  readonly components: readonly ChannelDeliveryComponent[];
}

export interface ChannelMessageItem {
  readonly delivery?: ChannelDelivery | null;
  readonly attachments?: readonly ChannelAttachment[];
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
    readonly attachments?: readonly OutboundAttachment[];
  };
}

export interface SendChannelMessageRequest {
  readonly conversation_id: string;
  readonly message: {
    readonly text?: string;
    readonly metadata?: Record<string, unknown>;
    readonly attachments?: readonly OutboundAttachment[];
  };
  readonly idempotency_key?: string;
}

export interface SendChannelMessageResponse {
  readonly message_id: string;
  /** Platform acceptance receipt, not proof that the recipient saw the message. */
  readonly platform_message_id?: string;
}

export type MediaKind = "image" | "file" | "audio" | "video";
export interface MediaCapabilities { readonly inbound: readonly MediaKind[]; readonly outbound: readonly MediaKind[] }
export interface OutboundAttachment {
  readonly kind: MediaKind;
  readonly source: { readonly type: "url"; readonly url: string } | { readonly type: "base64"; readonly data: string };
  readonly filename?: string;
  readonly mime_type?: string;
  readonly caption?: string;
}
export interface ChannelAttachment {
  readonly content_type: string;
  readonly url: string;
  readonly download_url: string;
  readonly platform_message_id?: string | null;
  readonly file_key?: string | null;
  readonly image_key?: string | null;
  readonly filename?: string | null;
  readonly mime_type?: string | null;
  readonly size_bytes?: number | null;
}
export interface ChannelRegistrationField {
  readonly hint: string | null;
  readonly name: string;
  readonly label: string;
  readonly secret: boolean;
  readonly required: boolean;
  readonly patchable: boolean;
  readonly clearable: boolean;
  readonly storage: string;
  readonly webhook_secret: boolean;
  readonly platform_fallback: string | null;
}
export interface ChannelPlatformDescriptor {
  readonly platform: ChannelPlatform;
  readonly display_name: string;
  readonly enabled: boolean;
  readonly managed_only: boolean;
  readonly managed_only_message: string;
  readonly ingestion: { readonly mode: "webhook" } | { readonly mode: "poll"; readonly min_interval_secs: number };
  readonly registration: {
    readonly documentation_url?: string | null;
    readonly fields: readonly ChannelRegistrationField[];
    readonly extra_fields: readonly ChannelRegistrationField[];
    readonly token_fields: readonly string[];
    readonly required_suffix: string;
    readonly automatic_webhook: boolean;
    readonly webhook_ingestion: boolean;
    readonly webhook_secret_label: string | null;
    readonly create_response_status: string;
    readonly setup_instructions: readonly string[];
  };
  readonly managed_onboarding: { readonly flow: string; readonly provider: string; readonly bootstrap_fields: readonly string[]; readonly completion_fields: readonly string[] } | null;
  readonly platform_credentials: { readonly provider: string; readonly configured: boolean } | null;
  readonly capabilities: OutboundCapabilities;
  readonly webhook_path: string | null;
}
export interface ChannelPlatformsResponse { readonly platforms: readonly ChannelPlatformDescriptor[] }
