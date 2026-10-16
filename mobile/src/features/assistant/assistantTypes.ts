import { z } from "zod";

export const conversationIdSchema = z.string().regex(/^nyxa-[a-f0-9]{32}$/);
const activitySchema = z.object({
  id: z.string(),
  label: z.string(),
  status: z.enum(["running", "completed", "error"]),
  started_at: z.string(),
  ended_at: z.string().nullish(),
});
export const attachmentSchema = z.object({
  id: z.string(),
  content_type: z.string(),
  size: z.number(),
  label: z.string(),
  origin: z.string().optional(),
  pages: z.number().int().nonnegative().nullish(),
  expired: z.boolean().optional(),
  image_input: z.string().nullish(),
});
export type AssistantAttachment = z.infer<typeof attachmentSchema>;
export const conversationSchema = z.object({
  id: conversationIdSchema,
  title: z.string(),
  model: z.string(),
  created_at: z.string(),
  last_message_at: z.string(),
  message_count: z.number(),
  pending_acknowledgements: z.number().default(0),
  active_turn: z
    .object({
      turn_id: z.string(),
      started_at: z.string(),
      stop_requested: z.boolean().optional(),
      continuations: z.number().optional(),
      activities: z.array(activitySchema).default([]),
      attachments: z.array(attachmentSchema).default([]),
    })
    .nullable(),
  context_reset_reason: z.string().nullish(),
  context_reset_at: z.string().nullish(),
  role: z.enum(["orchestrator", "subagent"]).default("orchestrator"),
  agent: z
    .object({
      id: z.string(),
      name: z.string(),
      display_name: z.string().nullish(),
      kind: z.enum(["nyxbot", "specialist"]),
      destroyed: z.boolean().default(false),
    })
    .nullish(),
  channel: z.object({ platform: z.string() }).nullish(),
});
export const messageSchema = z.object({
  id: z.string(),
  seq: z.number().int().positive(),
  turn_id: z.string(),
  role: z.enum(["user", "assistant", "orchestrator", "event"]),
  text: z.string(),
  status: z.enum(["completed", "failed"]),
  error_code: z.string().nullable(),
  created_at: z.string(),
  activities: z.array(activitySchema).default([]),
  attachments: z.array(attachmentSchema).default([]),
  via: z.string().nullish(),
  steering: z.object({ outcome: z.string() }).nullish(),
});
export const acknowledgementSchema = z.object({
  id: z.string().uuid(),
  kind: z.enum(["service", "account", "action", "operations", "skills"]),
  status: z.enum(["pending", "allowed", "denied", "expired", "used"]),
  summary: z.string(),
  decider: z.enum(["user", "orchestrator"]).default("user"),
  decided_by: z.enum(["user", "orchestrator"]).nullish(),
  reason: z.string().nullish(),
  service_slug: z.string().nullable(),
  service_name: z.string().nullable(),
  tool_name: z.string().nullable(),
  created_at: z.string(),
  expires_at: z.string(),
  confirmation: z
    .object({
      risk: z.enum(["read", "change", "sensitive", "destructive"]),
      reason: z.string(),
      access_mode: z.enum(["autopilot", "nyxmode", "ask_every_time"]).nullish(),
    })
    .nullish(),
  continuation_owner: z.literal("server").nullish(),
  trigger_run_id: z.string().nullish(),
  authored_skill: z
    .object({
      agent_id: z.string(),
      proposal_id: z.string(),
      revision: z.number().int().nonnegative().optional(),
      skills_revision: z.number().int().nonnegative().optional(),
    })
    .nullish(),
});
export const historySchema = z.object({
  conversation: conversationSchema,
  messages: z.array(messageSchema),
  acknowledgements: z.array(acknowledgementSchema).default([]),
  approvals: z
    .array(
      z.object({
        id: z.string(),
        service_name: z.string(),
        summary: z.string(),
        expires_at: z.string(),
      }),
    )
    .default([]),
  waiting: z
    .array(z.object({ title: z.string(), detail: z.string().nullish() }))
    .default([]),
  before_seq: z.number().int().positive().nullable(),
});
export const indexSchema = z.object({
  conversations: z.array(conversationSchema),
  next_cursor: z.string().nullable(),
});
export const modelListSchema = z.array(
  z.object({
    id: z.string(),
    label: z.string(),
    available: z.boolean().default(true),
    description: z.string().nullish(),
  }),
);
const block = z.object({
  type: z.literal("text"),
  block_id: z.string(),
  text: z.string(),
});
const cursor = { cursor: z.number().int().positive() };
export const turnEventSchema = z.discriminatedUnion("event", [
  z.object({
    ...cursor,
    event: z.literal("turn.status"),
    conversation_id: conversationIdSchema,
    turn_id: z.string(),
    status: z.enum(["running", "waiting"]),
  }),
  z.object({
    ...cursor,
    event: z.literal("turn.continuing"),
    turn_id: z.string(),
    continuation: z.number().int().positive(),
  }),
  z.object({
    ...cursor,
    event: z.literal("turn.notice"),
    code: z.string(),
    message: z.string(),
  }),
  z.object({
    ...cursor,
    event: z.literal("message.started"),
    message_id: z.string(),
    role: z.literal("assistant"),
  }),
  z.object({
    ...cursor,
    event: z.literal("block.started"),
    message_id: z.string(),
    block_id: z.string(),
    index: z.number().int().nonnegative(),
    block,
  }),
  z.object({
    ...cursor,
    event: z.literal("block.delta"),
    block_id: z.string(),
    text: z.string(),
  }),
  z.object({
    ...cursor,
    event: z.literal("block.completed"),
    block_id: z.string(),
    block,
  }),
  z.object({
    ...cursor,
    event: z.literal("message.completed"),
    message_id: z.string(),
  }),
  z.object({
    ...cursor,
    event: z.literal("turn.completed"),
    turn_id: z.string(),
    status: z.enum(["completed", "failed", "cancelled"]),
    error: z.object({ code: z.string(), message: z.string() }).nullable(),
  }),
]);
export type AssistantConversation = z.infer<typeof conversationSchema>;
export type AssistantHistory = z.infer<typeof historySchema>;
export type AssistantMessage = z.infer<typeof messageSchema>;
export type AssistantAcknowledgement = z.infer<typeof acknowledgementSchema>;
export type AssistantModel = z.infer<typeof modelListSchema>[number];
export type AssistantTurnEvent = z.infer<typeof turnEventSchema>;
