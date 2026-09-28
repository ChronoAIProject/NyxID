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

/// An image a tool returned during a turn, fetched from the owner-only
/// attachment route.
export const nyxAgentAttachmentSchema = z.object({
  id: z.string(),
  content_type: z.enum(["image/png", "image/jpeg", "image/gif", "image/webp"]),
  size: z.number().int().nonnegative(),
  label: z.string(),
});
export type NyxAgentAttachment = z.infer<typeof nyxAgentAttachmentSchema>;

/// Every thread belongs to an agent: the owner's NyxBot or a specialist.
export const assistantAgentKindSchema = z.enum(["nyxbot", "specialist"]);
export type AssistantAgentKind = z.infer<typeof assistantAgentKindSchema>;
export const assistantAgentStatusSchema = z.enum(["running", "idle", "destroyed"]);
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
export const nyxAgentConversationSchema = z.object({
  id: conversationId,
  title: z.string(),
  model: z.string(),
  created_at: z.string(),
  last_message_at: z.string(),
  message_count: z.number().int().nonnegative(),
  pending_acknowledgements: z.number().int().nonnegative().default(0),
  active_turn: z
    .object({
      turn_id: z.string(),
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
  channel: z.object({ platform: z.string() }).nullable().default(null),
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
});
export const nyxAgentAcknowledgementSchema = z.object({
  id: z.string().uuid(),
  kind: z.enum(["service", "account", "action"]),
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

export const nyxAgentHistorySchema = z.object({
  conversation: nyxAgentConversationSchema,
  messages: z.array(nyxAgentMessageSchema),
  acknowledgements: z.array(nyxAgentAcknowledgementSchema).default([]),
  approvals: z.array(nyxAgentApprovalSchema).default([]),
  before_seq: z.number().int().positive().nullable(),
});
export const nyxAgentIndexSchema = z.object({
  conversations: z.array(nyxAgentConversationSchema),
  next_cursor: z.string().nullable(),
});
export const nyxAgentModelsSchema = z.array(z.object({ id: z.string(), label: z.string() }));
const block = z.object({ type: z.literal("text"), block_id: z.string(), text: z.string() });
const base = z.object({ cursor: z.number().int().positive() });
export const nyxAgentEventSchema = z.discriminatedUnion("event", [
  base.extend({
    event: z.literal("turn.status"),
    conversation_id: conversationId,
    turn_id: z.string(),
    status: z.enum(["running", "waiting"]),
  }),
  base.extend({
    event: z.literal("turn.notice"),
    code: z.literal("context_reset"),
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
  skip_destructive_confirmation: z.boolean(),
  max_live_subagents: z.number().int().nonnegative(),
  max_concurrent_subagent_turns: z.number().int().nonnegative(),
  max_live_subagents_limit: z.number().int().nonnegative(),
  max_concurrent_subagent_turns_limit: z.number().int().positive(),
});
export type NyxAgentSettings = z.infer<typeof nyxAgentSettingsSchema>;
export type NyxAgentSettingsUpdate = Partial<
  Pick<
    NyxAgentSettings,
    "skip_destructive_confirmation" | "max_live_subagents" | "max_concurrent_subagent_turns"
  >
>;

/** The settings form. Confirmation is presented positively, the API stores the opt-out. */
export function nyxAgentSettingsFormSchema(limits: {
  readonly max_live_subagents_limit: number;
  readonly max_concurrent_subagent_turns_limit: number;
}) {
  const whole = (min: number, max: number) =>
    z
      .number({ error: "Enter a whole number" })
      .int("Enter a whole number")
      .min(min, `Must be at least ${String(min)}`)
      .max(max, `Must be at most ${String(max)}`);
  return z.object({
    confirm_destructive: z.boolean(),
    max_live_subagents: whole(0, limits.max_live_subagents_limit),
    max_concurrent_subagent_turns: whole(1, limits.max_concurrent_subagent_turns_limit),
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

export const assistantAgentSchema = z.object({
  id: z.string(),
  kind: assistantAgentKindSchema,
  name: z.string(),
  description: z.string().default(""),
  specialty: z.string().nullable().default(null),
  created_by: z.enum(["user", "nyxbot"]).catch("user"),
  status: assistantAgentStatusSchema,
  /** Granted service slugs (specialists). */
  services: z.array(z.string()).default([]),
  account_read: z.boolean().default(false),
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

/** "New agent": a specialist with its role and starting grants. */
export const assistantAgentCreateSchema = z.object({
  name: agentName,
  description: agentDescription,
  services: z.array(z.string()),
  account_read: z.boolean(),
});
export type AssistantAgentCreate = z.infer<typeof assistantAgentCreateSchema>;

/** Editing an agent: NyxBot keeps its name and its description is optional. */
export function assistantAgentProfileSchema(kind: AssistantAgentKind) {
  return kind === "nyxbot"
    ? z.object({
        name: z.string(),
        description: z.string().trim().max(2048, "Keep the notes under 2048 characters"),
      })
    : z.object({ name: agentName, description: agentDescription });
}
export type AssistantAgentProfile = { name: string; description: string };

export const assistantAgentGrantsSchema = z.object({
  services: z.array(z.string()),
  account_read: z.boolean(),
});
export type AssistantAgentGrants = z.infer<typeof assistantAgentGrantsSchema>;

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
export const nyxAgentChannelLinkedSchema = z.object({
  channel_agent_id: z.string(),
  /** The linked agent's name. */
  agent: z.string(),
  changed: z.boolean(),
});
