import { z } from "zod";

/** Metadata-only snapshot of the reply a vote is about (A1). Rows written
 * before A1 have no context; every field is optional for older servers. */
export const feedbackReplyContextSchema = z.object({
  surface: z.string().nullable().optional(),
  agent_id: z.string().nullable().optional(),
  agent_kind: z.string().nullable().optional(),
  model: z.string().nullable().optional(),
  access_mode: z.string().nullable().optional(),
  reply_status: z.string().nullable().optional(),
  error_code: z.string().nullable().optional(),
  origin: z.string().nullable().optional(),
  via: z.string().nullable().optional(),
  turn_id: z.string().nullable().optional(),
  reply_seq: z.number().nullable().optional(),
  reply_created_at: z.string().nullable().optional(),
  latency_ms: z.number().nullable().optional(),
  tool_calls: z.number().optional(),
  tool_errors: z.number().optional(),
  tool_labels: z.array(z.string()).optional(),
  machine_used: z.boolean().optional(),
  attachments: z.number().optional(),
  steered: z.boolean().optional(),
  voice: z.boolean().optional(),
  reply_chars: z.number().optional(),
  context_reset_since_reply: z.boolean().optional(),
  org_group: z.boolean().optional(),
});

export const assistantFeedbackAdminSchema = z.object({
  counts: z.array(z.object({ day: z.string(), up: z.number(), down: z.number() })),
  recent: z.array(
    z.object({
      message_id: z.string(),
      vote: z.enum(["up", "down"]),
      comment: z.string().nullable().optional(),
      context: feedbackReplyContextSchema.nullable().optional(),
      created_at: z.string(),
      updated_at: z.string(),
    }),
  ),
  next_cursor: z.string().nullable().optional(),
});

export type FeedbackReplyContext = z.infer<typeof feedbackReplyContextSchema>;
export type AssistantFeedbackAdmin = z.infer<typeof assistantFeedbackAdminSchema>;
