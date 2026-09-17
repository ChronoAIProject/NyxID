import { z } from "zod";

export const nyxAgentAccessModeSchema = z.enum(["ask", "full"]);
export type NyxAgentAccessMode = z.infer<typeof nyxAgentAccessModeSchema>;

export const nyxAgentTitleSchema = z.object({ title: z.string().trim().min(1).max(200) });

export const nyxAgentConversationSchema = z.object({
  id: z.string().regex(/^nyxa-[a-f0-9]{32}$/),
  title: z.string(),
  model: z.string(),
  access_mode: nyxAgentAccessModeSchema.default("ask"),
  created_at: z.string(),
  last_message_at: z.string(),
  message_count: z.number().int().nonnegative(),
  pending_acknowledgements: z.number().int().nonnegative().default(0),
  active_turn: z.object({ turn_id: z.string(), started_at: z.string() }).nullable(),
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

export const nyxAgentHistorySchema = z.object({
  conversation: nyxAgentConversationSchema,
  messages: z.array(nyxAgentMessageSchema),
  acknowledgements: z.array(nyxAgentAcknowledgementSchema).default([]),
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
