import { z } from "zod";

export const nyxAgentAccessModeSchema = z.enum(["ask", "full"]);
export type NyxAgentAccessMode = z.infer<typeof nyxAgentAccessModeSchema>;

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

export const nyxAgentConversationSchema = z.object({
  id: z.string().regex(/^nyxa-[a-f0-9]{32}$/),
  title: z.string(),
  model: z.string(),
  access_mode: nyxAgentAccessModeSchema.default("ask"),
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
});
export const nyxAgentMessageSchema = z.object({
  id: z.string(),
  seq: z.number().int().positive(),
  turn_id: z.string(),
  role: z.enum(["user", "assistant"]),
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
    conversation_id: z.string().regex(/^nyxa-[a-f0-9]{32}$/),
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
