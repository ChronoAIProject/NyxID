import { z } from "zod";

export const orgGuestConversationId = /^org-guest-[a-f0-9]{32}$/;

export const orgGuestConversationSchema = z.object({
  id: z.string().regex(orgGuestConversationId),
  agent_id: z.string(),
  title: z.string(),
  read_only: z.literal(true),
  created_at: z.string(),
  last_message_at: z.string(),
  channel: z.object({
    nyxbot_channel_id: z.string(),
    platform: z.string(),
    bot_label: z.string(),
    chat_id: z.string(),
    chat_kind: z.string(),
    chat_title: z.string().nullish(),
    parent_chat_id: z.string().nullish(),
    parent_title: z.string().nullish(),
    thread_id: z.string().nullish(),
  }),
});

export const orgGuestIndexSchema = z.object({
  conversations: z.array(orgGuestConversationSchema),
  next_cursor: z.string().nullable(),
});

export const orgGuestHistorySchema = z.object({
  conversation: orgGuestConversationSchema,
  messages: z.array(z.object({
    seq: z.number().int(),
    role: z.enum(["user", "assistant"]),
    status: z.enum(["completed", "failed"]),
    text: z.string(),
    created_at: z.string(),
    error_code: z.string().optional(),
    delivery_status: z.string().optional(),
    tool_failures: z.array(z.object({
      kind: z.string(),
      reason: z.string().nullable(),
      status: z.number().int().nullable(),
    })).optional(),
  })),
  before_seq: z.number().int().nullable(),
});

export type OrgGuestConversation = z.infer<typeof orgGuestConversationSchema>;
