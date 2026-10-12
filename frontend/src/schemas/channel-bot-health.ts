import { z } from "zod";

const errorSchema = z.object({
  code: z.string(),
  /** Stable failure code (`rate_limited`, `scopes_missing`, ...). */
  reason: z.string().nullable(),
  at: z.string(),
});

export const channelBotDiagnosticsSchema = z.object({
  last_inbound_at: z.string().nullable(),
  last_inbound_source: z.string().nullable(),
  last_outbound_at: z.string().nullable(),
  last_outbound_error: errorSchema.nullable(),
  last_webhook_at: z.string().nullable(),
  last_webhook_error: errorSchema.nullable(),
  subscription_state: z
    .object({
      registered: z.boolean(),
      checked_at: z.string(),
      last_error: z.string().nullable(),
    })
    .nullable(),
  consecutive_errors: z.number().int().nonnegative(),
  failure_reason: z.string().nullable(),
});
export type ChannelBotDiagnostics = z.infer<typeof channelBotDiagnosticsSchema>;

export const RECOMMENDED_ACTIONS = [
  "none",
  "reconnect",
  "resubscribe",
  "verify",
  "wait_rate_limit",
  "enable_billing",
  "contact_admin",
  "gateway",
] as const;

export const channelBotHealthSchema = z.object({
  id: z.string(),
  platform: z.string(),
  label: z.string(),
  platform_bot_username: z.string(),
  user_id: z.string(),
  mode: z.enum(["webhook", "poll"]),
  status: z.string(),
  webhook_registered: z.boolean(),
  poll_error_count: z.number().int().nonnegative(),
  last_polled_at: z.string().nullable(),
  next_poll_at: z.string().nullable(),
  error: z.string().nullable(),
  recommended_action: z.enum(RECOMMENDED_ACTIONS),
  recommended_action_text: z.string(),
  /** Whether the caller may run the recovery action. */
  can_write: z.boolean(),
  diagnostics: channelBotDiagnosticsSchema,
});
export type ChannelBotHealth = z.infer<typeof channelBotHealthSchema>;
