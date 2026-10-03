import { DEFAULT_SCHEDULE_MINIMUM_MINUTES } from "@/lib/automation-limits";
import { z } from "zod";

const conversationId = z.string().regex(/^nyxa-[a-f0-9]{32}$/);

export const nyxAgentTitleSchema = z.object({ title: z.string().trim().min(1).max(200) });

/// A tool call the assistant made during a turn: identifier and status only.
export const nyxAgentTurnActivitySchema = z.object({
  id: z.string(),
  label: z.string(),
  status: z.enum(["running", "completed", "error"]),
  started_at: z.string(),
  ended_at: z.string().nullable().default(null),
});
export type NyxAgentTurnActivity = z.infer<typeof nyxAgentTurnActivitySchema>;

/// A tool image or human upload, fetched from the owner-only attachment route.
export const nyxAgentAttachmentSchema = z.object({
  id: z.string(),
  content_type: z.enum([
    "image/png", "image/jpeg", "image/gif", "image/webp", "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "text/plain", "text/markdown", "text/csv", "application/json",
  ]),
  origin: z.enum(["tool", "user_upload", ""]).optional(),
  pages: z.number().int().nonnegative().nullable().optional(),
  expired: z.boolean().optional(),
  image_input: z.enum(["sent", "unavailable"]).nullable().optional(),
  size: z.number().int().nonnegative(),
  label: z.string(),
});
export type NyxAgentAttachment = z.infer<typeof nyxAgentAttachmentSchema>;

/// Every thread belongs to an agent: the owner's NyxBot or a specialist.
export const assistantAgentKindSchema = z.enum(["nyxbot", "specialist"]);
export type AssistantAgentKind = z.infer<typeof assistantAgentKindSchema>;
export const assistantAgentStatusSchema = z.enum(["running", "idle", "destroyed"]);

/**
 * What guests (members of the agent's chats other than you) may do with one
 * of a specialist's services: look things up only, use it (look up, create and
 * act, but never change or delete what exists; the default), or everything the
 * specialist may.
 */
export const assistantGuestAccessSchema = z.enum(["read", "use", "all"]);
export type AssistantGuestAccess = z.infer<typeof assistantGuestAccessSchema>;
export type AssistantAgentStatus = z.infer<typeof assistantAgentStatusSchema>;

/// The agent a thread belongs to, as carried on each conversation.
export const nyxAgentConversationAgentSchema = z.object({
  id: z.string(),
  kind: assistantAgentKindSchema,
  name: z.string(),
  /** A destroyed agent's threads are read-only. */
  destroyed: z.boolean().default(false),
});
export type NyxAgentConversationAgent = z.infer<typeof nyxAgentConversationAgentSchema>;

// `access_mode` is still returned (always "full") for older clients; every
// chat runs with Full access, so it is deliberately not parsed.
/** The channel bot and chat a thread answers. */
export const nyxAgentChannelOriginSchema = z.object({
  platform: z.string(),
  /** The channel bot connection; groups the bot's chats in the sidebar. */
  channel_agent_id: z.string().nullable().default(null),
  bot_label: z.string().nullable().default(null),
  chat_id: z.string().nullable().default(null),
  /** `private`, `group` or `channel`; null for chats not seen since 0.35. */
  chat_kind: z.string().nullable().default(null),
  chat_title: z.string().nullable().default(null),
});
export type NyxAgentChannelOrigin = z.infer<typeof nyxAgentChannelOriginSchema>;
export const nyxAgentConversationSchema = z.object({
  id: conversationId,
  title: z.string(),
  title_source: z.enum(["provisional", "generated", "user"]).optional(),
  model: z.string(),
  created_at: z.string(),
  last_message_at: z.string(),
  message_count: z.number().int().nonnegative(),
  pending_acknowledgements: z.number().int().nonnegative().default(0),
  active_turn: z
    .object({
      turn_id: z.string(),
      continuations: z.number().int().nonnegative().optional(),
      started_at: z.string(),
      activities: z.array(nyxAgentTurnActivitySchema).default([]),
      attachments: z.array(nyxAgentAttachmentSchema).default([]),
    })
    .nullable(),
  context_reset_at: z.string().nullable(),
  /** `orchestrator` on NyxBot's threads, `subagent` on a specialist's. */
  role: z.enum(["orchestrator", "subagent"]).default("orchestrator"),
  /** The owning agent; null only when the server could not resolve it. */
  agent: nyxAgentConversationAgentSchema.nullable().default(null),
  /** Wake-up events queued for this thread's next turn. */
  pending_events: z.number().int().nonnegative().default(0),
  /** Set on threads that answer one of the user's channel bots. */
  channel: nyxAgentChannelOriginSchema.nullable().default(null),
});
/**
 * `orchestrator` is an instruction NyxBot sent to a specialist; `event` is a
 * server-authored NyxID notice that woke the agent.
 */
export const nyxAgentMessageRoleSchema = z.enum(["user", "assistant", "orchestrator", "event"]);
export type NyxAgentMessageRole = z.infer<typeof nyxAgentMessageRoleSchema>;
export const nyxAgentMessageSchema = z.object({
  id: z.string(),
  seq: z.number().int().positive(),
  turn_id: z.string(),
  role: nyxAgentMessageRoleSchema,
  text: z.string(),
  status: z.enum(["completed", "failed"]),
  error_code: z.string().nullable(),
  created_at: z.string(),
  activities: z.array(nyxAgentTurnActivitySchema).default([]),
  attachments: z.array(nyxAgentAttachmentSchema).default([]),
  /** A user message written in a chat app: its platform. */
  via: z.string().nullish(),
});
export const nyxAgentAcknowledgementSchema = z.object({
  trigger_run_id: z.string().nullable().optional(),
  id: z.string().uuid(),
  kind: z.enum(["service", "account", "action", "operations", "skills"]),
  status: z.enum(["pending", "allowed", "denied", "expired", "used"]),
  summary: z.string(),
  /** `orchestrator`: a specialist's request that NyxBot decides (the user may too). */
  decider: z.enum(["user", "orchestrator"]).default("user"),
  decided_by: z.enum(["user", "orchestrator"]).nullable().default(null),
  reason: z.string().nullable().default(null),
  service_slug: z.string().nullable(),
  service_name: z.string().nullable(),
  tool_name: z.string().nullable(),
  created_at: z.string(),
  decided_at: z.string().nullable(),
  expires_at: z.string(),
});
export type NyxAgentAcknowledgement = z.infer<typeof nyxAgentAcknowledgementSchema>;

/// A pending proxy approval raised by the chat's key; decided through the
/// ordinary approvals API.
export const nyxAgentApprovalSchema = z.object({
  id: z.string(),
  service_slug: z.string(),
  service_name: z.string(),
  summary: z.string(),
  approval_mode: z.enum(["per_request", "grant"]),
  agent_key_prefix: z.string().default(""),
  created_at: z.string(),
  expires_at: z.string(),
});
export type NyxAgentApproval = z.infer<typeof nyxAgentApprovalSchema>;

/// Something outside the chat the thread is waiting for (a bot being
/// created, a service being connected, the owner verifying a chat app).
/// NyxID resumes the thread by itself when it happens.
export const nyxAgentWaitingSchema = z.object({
  kind: z.string(),
  title: z.string(),
  /** What NyxID has seen so far, when that explains a long wait. */
  detail: z.string().nullable().default(null),
  since: z.string(),
  expires_at: z.string().nullable().default(null),
});
export type NyxAgentWaiting = z.infer<typeof nyxAgentWaitingSchema>;

export const nyxAgentHistorySchema = z.object({
  conversation: nyxAgentConversationSchema,
  messages: z.array(nyxAgentMessageSchema),
  acknowledgements: z.array(nyxAgentAcknowledgementSchema).default([]),
  approvals: z.array(nyxAgentApprovalSchema).default([]),
  waiting: z.array(nyxAgentWaitingSchema).default([]),
  before_seq: z.number().int().positive().nullable(),
});
export const nyxAgentIndexSchema = z.object({
  conversations: z.array(nyxAgentConversationSchema),
  next_cursor: z.string().nullable(),
});
const block = z.object({ type: z.literal("text"), block_id: z.string(), text: z.string() });
const base = z.object({ cursor: z.number().int().positive() });
export const nyxAgentEventSchema = z.discriminatedUnion("event", [
  base.extend({
    event: z.literal("turn.status"),
    conversation_id: conversationId,
    turn_id: z.string(),
    status: z.enum(["running", "waiting"]),
  }),
  base.extend({ event: z.literal("turn.continuing"), turn_id: z.string(), continuation: z.number().int().positive() }),
  base.extend({
    event: z.literal("turn.notice"),
    code: z.enum(["context_reset", "image_input_unavailable"]),
    message: z.string(),
  }),
  base.extend({
    event: z.literal("message.started"),
    message_id: z.string(),
    role: z.literal("assistant"),
  }),
  base.extend({
    event: z.literal("block.started"),
    message_id: z.string(),
    block_id: z.string(),
    index: z.number().int().nonnegative(),
    block,
  }),
  base.extend({ event: z.literal("block.delta"), block_id: z.string(), text: z.string() }),
  base.extend({ event: z.literal("block.completed"), block_id: z.string(), block }),
  base.extend({ event: z.literal("message.completed"), message_id: z.string() }),
  base.extend({
    event: z.literal("turn.completed"),
    turn_id: z.string(),
    status: z.enum(["completed", "failed", "cancelled"]),
    error: z.object({ code: z.string(), message: z.string() }).nullable(),
  }),
]);
export type NyxAgentConversation = z.infer<typeof nyxAgentConversationSchema>;
export type NyxAgentHistory = z.infer<typeof nyxAgentHistorySchema>;

export const nyxAgentSettingsSchema = z.object({
  timezone: z.string().nullable().optional(),
  schedule_minimum_minutes: z
    .number()
    .int()
    .min(DEFAULT_SCHEDULE_MINIMUM_MINUTES)
    .optional(),
  trigger_runs_per_hour: z.number().int().positive().optional(),
  trigger_runs_per_day: z.number().int().positive().optional(),
  max_auto_continuations: z.number().int().min(0).max(32).optional(),
  max_auto_continuations_limit: z.number().int().positive().optional(),
  skip_destructive_confirmation: z.boolean(),
  max_live_subagents: z.number().int().nonnegative(),
  max_concurrent_subagent_turns: z.number().int().nonnegative(),
  max_live_subagents_limit: z.number().int().nonnegative(),
  max_concurrent_subagent_turns_limit: z.number().int().positive(),
  /** Agent-to-agent hand-offs in a group per message you send (0 turns them off). */
  max_group_handoffs: z.number().int().nonnegative().default(6),
  /** Hand-offs per hour across all your groups. */
  max_group_handoffs_per_hour: z.number().int().nonnegative().default(60),
  max_group_handoffs_limit: z.number().int().nonnegative().default(24),
  max_group_handoffs_per_hour_limit: z
    .number()
    .int()
    .nonnegative()
    .default(600),
});
export type NyxAgentSettings = z.infer<typeof nyxAgentSettingsSchema>;
export type NyxAgentSettingsUpdate = Partial<
  Pick<
    NyxAgentSettings,
    | "timezone"
    | "schedule_minimum_minutes"
    | "trigger_runs_per_hour"
    | "trigger_runs_per_day"
    | "skip_destructive_confirmation"
    | "max_auto_continuations"
    | "max_live_subagents"
    | "max_concurrent_subagent_turns"
    | "max_group_handoffs"
    | "max_group_handoffs_per_hour"
  >
>;

/** The settings form. Confirmation is presented positively, the API stores the opt-out. */
export function nyxAgentSettingsFormSchema(limits: {
  readonly max_live_subagents_limit: number;
  readonly max_concurrent_subagent_turns_limit: number;
  readonly max_group_handoffs_limit: number;
  readonly max_group_handoffs_per_hour_limit: number;
}) {
  const whole = (min: number, max: number) =>
    z
      .number({ error: "Enter a whole number" })
      .int("Enter a whole number")
      .min(min, `Must be at least ${String(min)}`)
      .max(max, `Must be at most ${String(max)}`);
  return z.object({
    max_auto_continuations: whole(0, 32),
    confirm_destructive: z.boolean(),
    max_live_subagents: whole(0, limits.max_live_subagents_limit),
    max_concurrent_subagent_turns: whole(1, limits.max_concurrent_subagent_turns_limit),
    max_group_handoffs: whole(0, limits.max_group_handoffs_limit),
    max_group_handoffs_per_hour: whole(0, limits.max_group_handoffs_per_hour_limit),
  });
}
export type NyxAgentSettingsForm = z.infer<ReturnType<typeof nyxAgentSettingsFormSchema>>;

/** A specialist's permission request awaiting NyxBot (or the user). */
export const assistantAgentRequestSchema = z.object({
  request_id: z.string(),
  /** The requesting agent's name. */
  agent: z.string(),
  agent_id: z.string().nullable().default(null),
  /** The thread holding the request card. */
  conversation_id: conversationId,
  kind: z.string(),
  service_slug: z.string().nullable().default(null),
  summary: z.string(),
  requested_by: z.string().nullable().default(null),
  expires_at: z.string(),
});
export type AssistantAgentRequest = z.infer<typeof assistantAgentRequestSchema>;

export const ASSISTANT_AGENT_DISPLAY_NAME_MAX = 40;
export const ASSISTANT_AGENT_PERSONA_MAX = 2000;

export const assistantAgentSchema = z.object({
  owner_id: z.string().optional(),
  owner_name: z.string().nullable().optional(),
  owner_kind: z.enum(["person", "org"]).optional(),
  org_role: z.enum(["admin", "member", "viewer"]).nullable().optional(),
  can_maintain: z.boolean().optional(),
  can_use: z.boolean().optional(),
  id: z.string(),
  kind: assistantAgentKindSchema,
  /** The @handle (fixed "NyxBot" for NyxBot). */
  name: z.string(),
  /** A friendly name the user chose ("Luna"); shown instead of the handle. */
  display_name: z.string().nullable().default(null),
  /** Personality and tone the user asked for; style only, never authority. */
  persona: z.string().nullable().default(null),
  description: z.string().default(""),
  specialty: z.string().nullable().default(null),
  created_by: z.enum(["user", "nyxbot"]).catch("user"),
  status: assistantAgentStatusSchema,
  /** Granted service slugs (specialists). */
  services: z.array(z.string()).default([]),
  machines: z.array(z.string()).optional(),
  logins: z.array(z.string()).optional(),
  account_read: z.boolean().default(false),
  /** What guests (other members of the agent's chats) may do with each service. */
  guest_access: z.record(z.string(), assistantGuestAccessSchema.catch("use")).default({}),
  pending_requests: z.array(assistantAgentRequestSchema).default([]),
  last_reply: z
    .object({
      seq: z.number().int(),
      status: z.string(),
      text: z.string(),
      created_at: z.string(),
    })
    .nullable()
    .default(null),
  home_conversation_id: conversationId.nullable().default(null),
  memory_count: z.number().int().nonnegative().default(0),
  created_at: z.string(),
  last_active_at: z.string(),
  destroyed_at: z.string().nullable().default(null),
  pending_acknowledgements: z.number().int().nonnegative().default(0),
  /** Channel bots that reach this agent. */
  channels: z
    .array(
      z.object({
        id: z.string(),
        platform: z.string(),
        bot_label: z.string(),
        status: z.string(),
      }),
    )
    .default([]),
});
export type AssistantAgent = z.infer<typeof assistantAgentSchema>;

export const assistantAgentListSchema = z.object({
  agents: z.array(assistantAgentSchema),
  limits: nyxAgentSettingsSchema,
});
export type AssistantAgentList = z.infer<typeof assistantAgentListSchema>;

export const assistantAgentMemoryNoteSchema = z.object({
  id: z.string(),
  text: z.string(),
  created_at: z.string(),
  updated_at: z.string(),
});
export type AssistantAgentMemoryNote = z.infer<typeof assistantAgentMemoryNoteSchema>;

export const assistantAgentDetailSchema = z.object({
  agent: assistantAgentSchema,
  memory: z.array(assistantAgentMemoryNoteSchema).default([]),
  threads: z
    .array(
      z.object({
        id: conversationId,
        title: z.string(),
        last_message_at: z.string(),
        channel: z.object({ platform: z.string() }).nullable().default(null),
        running: z.boolean().default(false),
      }),
    )
    .default([]),
});
export type AssistantAgentDetail = z.infer<typeof assistantAgentDetailSchema>;

export const assistantAgentCreatedSchema = z.object({
  id: z.string(),
  name: z.string(),
  home_conversation_id: conversationId,
});

export const ASSISTANT_AGENT_NAME = /^[a-z0-9][a-z0-9-]{0,31}$/;
const agentName = z
  .string()
  .trim()
  .regex(
    ASSISTANT_AGENT_NAME,
    "Use 1 to 32 lowercase letters, digits or hyphens, starting with a letter or digit",
  );
const agentDescription = z
  .string()
  .trim()
  .min(1, "Describe what this agent does")
  .max(2048, "Keep the description under 2048 characters");
/** Empty clears it. */
const agentDisplayName = z
  .string()
  .trim()
  .max(
    ASSISTANT_AGENT_DISPLAY_NAME_MAX,
    `Keep the display name under ${String(ASSISTANT_AGENT_DISPLAY_NAME_MAX)} characters`,
  );
/** Empty clears it. */
const agentPersona = z
  .string()
  .trim()
  .max(
    ASSISTANT_AGENT_PERSONA_MAX,
    `Keep the persona under ${String(ASSISTANT_AGENT_PERSONA_MAX)} characters`,
  );

/** "New agent": a specialist with its role, optional style and starting grants. */
export const assistantAgentCreateSchema = z.object({
  org: z.string().optional(),
  name: agentName,
  display_name: agentDisplayName.optional(),
  description: agentDescription,
  persona: agentPersona.optional(),
  machines: z.array(z.string()).max(64).optional(),
  logins: z.array(z.string()).max(64).optional(),
  services: z.array(z.string()),
  account_read: z.boolean(),
});
export type AssistantAgentCreate = z.infer<typeof assistantAgentCreateSchema>;

/** Editing an agent: NyxBot keeps its name and its description is optional. */
export function assistantAgentProfileSchema(kind: AssistantAgentKind) {
  return kind === "nyxbot"
    ? z.object({
        name: z.string(),
        display_name: agentDisplayName,
        description: z.string().trim().max(2048, "Keep the notes under 2048 characters"),
        persona: agentPersona,
      })
    : z.object({
        name: agentName,
        display_name: agentDisplayName,
        description: agentDescription,
        persona: agentPersona,
      });
}
export type AssistantAgentProfile = {
  name: string;
  display_name: string;
  description: string;
  persona: string;
};

export const assistantAgentGrantsSchema = z.object({
  machines: z.array(z.string()).optional(),
  logins: z.array(z.string()).optional(),
  services: z.array(z.string()),
  account_read: z.boolean(),
  /** Guest access by service slug; services left out keep their level. */
  guest_access: z.record(z.string(), assistantGuestAccessSchema),
});
/** What the grants endpoint takes: levels only for services whose level changed. */
export const assistantAgentGrantsRequestSchema = z.object({
  machines: z.array(z.string()).optional(),
  logins: z.array(z.string()).optional(),
  services: z.array(z.string()),
  account_read: z.boolean(),
  guest_access: z.record(z.string(), assistantGuestAccessSchema).optional(),
});
export type AssistantAgentGrants = z.infer<typeof assistantAgentGrantsSchema>;
export type AssistantAgentGrantsRequest = z.infer<typeof assistantAgentGrantsRequestSchema>;

export const assistantAgentDestroyedSchema = z.object({ id: z.string(), destroyed_at: z.string() });

/** A channel bot that reaches one of the owner's agents. */
export const nyxAgentChannelAgentSchema = z.object({
  id: z.string(),
  channel_bot_id: z.string(),
  platform: z.string(),
  bot_label: z.string(),
  bot_username: z.string().nullable().default(null),
  transport: z.string(),
  /** `pending`, `active` or `failed`. */
  status: z.string(),
  last_error: z.string().nullable().default(null),
  owner_linked: z.boolean(),
  /** The agent this bot reaches; null means the owner's NyxBot. */
  agent_id: z.string().nullable().default(null),
  /** The organization that owns the bot; null for the user's own bot. */
  org_id: z.string().nullable().default(null),
  /** Who may talk to the agent in private chats with the bot. */
  private_chats: z.enum(["owner", "everyone"]).catch("owner"),
  /** `ok` or `failing` once a message has been judged; null before. */
  delivery_status: z.string().nullable().default(null),
  delivery_error: z.string().nullable().default(null),
  /** Plain words for `delivery_error`. */
  delivery_reason: z.string().nullable().default(null),
  delivery_failed_at: z.string().nullable().default(null),
  /** While the owner has not verified: what NyxID saw from the bot. */
  inbound_hint: z.string().nullable().default(null),
  created_at: z.string(),
});
export type NyxAgentChannelAgent = z.infer<typeof nyxAgentChannelAgentSchema>;
export const nyxAgentChannelListSchema = z.object({
  channel_agents: z.array(nyxAgentChannelAgentSchema),
});
/** The one-time owner link. Only an https URL is ever opened. */
export const nyxAgentChannelLinkSchema = z.object({
  code: z.string(),
  url: z
    .string()
    .nullable()
    .default(null)
    .transform((value) => (value && /^https:\/\/[^\s]+$/.test(value) ? value : null)),
  expires_at: z.string(),
  instructions: z.string(),
});
export type NyxAgentChannelLink = z.infer<typeof nyxAgentChannelLinkSchema>;
export const nyxAgentChannelConnectSchema = z.object({
  channel_agent: nyxAgentChannelAgentSchema,
  link: nyxAgentChannelLinkSchema,
});
export type NyxAgentChannelConnect = z.infer<typeof nyxAgentChannelConnectSchema>;
/** A chat a channel bot is in: a private chat, group, channel or topic. */
export const nyxAgentChannelChatSchema = z.object({
  id: z.string(),
  channel_agent_id: z.string(),
  platform: z.string(),
  bot_label: z.string(),
  /** `private`, `group` or `channel`; null until the chat speaks again. */
  kind: z.string().nullable().default(null),
  title: z.string().nullable().default(null),
  /** The agent this chat reaches instead of the bot's; null: the bot's. */
  agent_id: z.string().nullable().default(null),
  /** `all` answers every message; `mention` only mentions and replies. */
  reply_mode: z.enum(["mention", "all"]).catch("mention"),
  /**
   * Who may talk to the agent: members too, or only the user (private
   * chats: the bot's setting). Groups open once the user has talked there.
   */
  members: z.enum(["everyone", "owner"]).catch("owner"),
  /** The user's explicit choice; null follows the default. */
  members_setting: z.enum(["everyone", "owner"]).nullable().catch(null).default(null),
  owner_seen: z.boolean().default(false),
  /** Private chats: the user's own chat with the bot. */
  owner: z.boolean().default(false),
  allow_posts: z.boolean().default(false),
  conversation_id: z.string().nullable().default(null),
  last_message_at: z.string().nullable().default(null),
});
export type NyxAgentChannelChat = z.infer<typeof nyxAgentChannelChatSchema>;
export const nyxAgentChannelChatListSchema = z.object({
  chats: z.array(nyxAgentChannelChatSchema),
});
export const nyxAgentChannelChatUpdatedSchema = z.object({
  chat: nyxAgentChannelChatSchema,
  warning: z.string().optional(),
  note: z.string().optional(),
});
export type NyxAgentChannelChatUpdated = z.infer<typeof nyxAgentChannelChatUpdatedSchema>;
export type NyxAgentChannelChatSettings = Partial<{
  reply_mode: "mention" | "all";
  /** `default` lets members talk once the user has talked there. */
  members: "everyone" | "owner" | "default";
  allow_posts: boolean;
  /** An agent ID, or `default` for the bot's agent. */
  agent_id: string;
}>;

export const nyxAgentChannelLinkedSchema = z.object({
  channel_agent_id: z.string(),
  /** The linked agent's name. */
  agent: z.string(),
  changed: z.boolean(),
});

/// Group chats: the owner plus 1..8 of their agents.
export const ASSISTANT_GROUP_MAX_MEMBERS = 8;
export const ASSISTANT_GROUP_NAME_MAX = 60;

export const assistantGroupMemberSchema = z.object({
  id: z.string(),
  name: z.string(),
  kind: assistantAgentKindSchema,
  destroyed: z.boolean().default(false),
  working: z.boolean().default(false),
});
export type AssistantGroupMember = z.infer<typeof assistantGroupMemberSchema>;

export const assistantGroupSchema = z.object({
  id: z.string(),
  name: z.string(),
  members: z.array(assistantGroupMemberSchema),
  /** Answers user messages that mention no one: NyxBot when it is a member. */
  lead_agent_id: z.string(),
  working_agent_ids: z.array(z.string()).default([]),
  message_count: z.number().int().nonnegative().default(0),
  last_message_at: z.string().nullable().default(null),
  created_at: z.string(),
});
export type AssistantGroup = z.infer<typeof assistantGroupSchema>;

export const assistantGroupListSchema = z.object({ groups: z.array(assistantGroupSchema) });

export const assistantGroupMessageSchema = z.object({
  attachments: z.array(nyxAgentAttachmentSchema).optional(),
  id: z.string(),
  seq: z.number().int().positive(),
  /** `notice` is a NyxID-authored system line (members joined, renamed, ...). */
  role: z.enum(["user", "agent", "notice"]),
  /** The speaking agent, set on `agent` messages. */
  agent: z
    .object({ id: z.string(), name: z.string(), kind: assistantAgentKindSchema })
    .nullable()
    .default(null),
  text: z.string(),
  created_at: z.string(),
});
export type AssistantGroupMessage = z.infer<typeof assistantGroupMessageSchema>;

/** A member's action card waiting for the owner; answered by posting its phrase. */
export const assistantGroupPendingActionSchema = z.object({
  conversation_id: z.string(),
  acknowledgement_id: z.string(),
  agent_id: z.string().nullable().optional(),
  summary: z.string(),
  /** "yes 1234": posting it confirms; "no 1234" cancels. */
  confirm_phrase: z.string().regex(/^yes \d{4}$/),
  expires_at: z.string(),
});
export type AssistantGroupPendingAction = z.infer<typeof assistantGroupPendingActionSchema>;

export const assistantGroupMessagesSchema = z.object({
  group: assistantGroupSchema,
  pending_actions: z.array(assistantGroupPendingActionSchema).default([]),
  /** Ascending by `seq`. */
  messages: z.array(assistantGroupMessageSchema),
  /** Pass as `before_seq` to read the next older page; null at the start. */
  before_seq: z.number().int().positive().nullable().default(null),
});
export type AssistantGroupMessages = z.infer<typeof assistantGroupMessagesSchema>;

export const assistantGroupPostedSchema = z.object({
  message: assistantGroupMessageSchema,
  addressed_agent_ids: z.array(z.string()).default([]),
});
export type AssistantGroupPosted = z.infer<typeof assistantGroupPostedSchema>;

/** "New group" and group settings: a name and its agents. */
export const assistantGroupFormSchema = z.object({
  name: z
    .string()
    .trim()
    .min(1, "Name the group")
    .max(ASSISTANT_GROUP_NAME_MAX, `Keep the name under ${String(ASSISTANT_GROUP_NAME_MAX)} characters`),
  member_agent_ids: z
    .array(z.string())
    .min(1, "Pick at least one agent")
    .max(ASSISTANT_GROUP_MAX_MEMBERS, `A group has at most ${String(ASSISTANT_GROUP_MAX_MEMBERS)} agents`),
});
export type AssistantGroupForm = z.infer<typeof assistantGroupFormSchema>;
export type AssistantGroupUpdate = Partial<AssistantGroupForm>;

export const ASSISTANT_MEMORY_NOTE_MAX = 500;
export const assistantMemoryNoteSchema = z.object({
  text: z
    .string()
    .trim()
    .min(1, "Enter a shared note")
    .max(ASSISTANT_MEMORY_NOTE_MAX),
});
