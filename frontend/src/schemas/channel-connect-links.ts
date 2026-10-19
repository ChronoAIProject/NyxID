import { z } from "zod";

export const channelConnectLinkSchema = z.object({
  id: z.string(),
  status: z.enum(["pending", "completed", "cancelled", "expired"]),
  platform: z.string(),
  label: z.string(),
  owner_id: z.string().nullable(),
  requested_by: z.string().nullable(),
  expires_at: z.string(),
  bot_id: z.string().nullable(),
  connection_id: z.string().nullable(),
  telegram_request_id: z.string().nullable(),
  telegram_requires_original_actor: z.boolean().optional(),
  callback_url: z.string().nullable(),
  last_error: z.string().nullable(),
  delivery_status: z.string().nullable(),
});

export type ChannelConnectLink = z.infer<typeof channelConnectLinkSchema>;
