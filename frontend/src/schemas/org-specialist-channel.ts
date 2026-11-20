import { z } from "zod";
import { agentConfigurationSchema, guestReadinessSchema } from "./assistant-nyxagent";

export const orgChannelSettingsSchema = z.object({
  channel_id: z.string(),
  agent_id: z.string().nullish(),
  grants_ready: z.boolean().default(false),
  binding_limits_guest_services: z.boolean().default(false),
  guest_policy_mode: z.enum(["legacy", "grants"]).default("legacy"),
  effective_guest_services: z
    .array(
      z.object({
        id: z.string(),
        label: z.string(),
        access: z.string(),
        limited_operations: z.boolean(),
      }),
    )
    .default([]),
  org_id: z.string(),
  can_manage: z.boolean(),
  guest_revision: z.string().default(""),
  channel_status: z.string().optional(),
  description: z.string().optional(),
  member_verified: z.boolean().optional(),
  guest_enabled: z.boolean(),
  delegated_service_ids: z.array(z.string()),
  daily_credits: z.string().nullable().optional(),
  default_daily_credits: z.string().optional(),
  spent_today_credits: z.string().optional(),
  reserved_today_credits: z.string().optional(),
  private_chats: z.enum(["owner", "everyone"]).optional(),
  health: z.object({ action: z.string(), text: z.string() }).nullish(),
  readiness_reason: z.string().nullable(),
  available_services: z.array(z.object({ id: z.string(), label: z.string() })),
});
export type OrgChannelSettings = z.infer<typeof orgChannelSettingsSchema>;
export const orgChannelGuestFormSchema = z.object({ enabled: z.boolean() });
export type OrgChannelGuestForm = z.infer<typeof orgChannelGuestFormSchema>;
export type OrgChannelGuestUpdate = OrgChannelGuestForm & {
  expected_revision?: string;
};
export const orgGuestAdoptionSchema = z.object({
  ready: z.boolean(),
  pending: z.boolean(),
  review: z.string(),
  bots: z.array(
    z.object({ id: z.string(), name: z.string(), status: z.string() }),
  ),
  services: z.array(
    z.object({
      id: z.string(),
      label: z.string(),
      access: z.enum(["none", "read", "use", "all"]),
      limited_operations: z.boolean(),
    }),
  ),
});
export const orgChannelChallengeSchema = z.object({
  id: z.string(),
  state: z.string(),
  code: z.string().optional(),
  expires_at: z.string().optional(),
  sender_label: z.string().nullish(),
});

export const orgChannelPreviewSchema = z.object({
  bot_id: z.string(), agent_id: z.string(), org_id: z.string(),
  guest_enabled: z.boolean(), private_chats: z.enum(["owner", "everyone"]),
  guest_policy_mode: z.enum(["legacy", "grants"]),
  description: z.string(),
  effective_guest_services: orgChannelSettingsSchema.shape.effective_guest_services,
  audience_readiness: z.object({ guest_readiness: guestReadinessSchema, configuration: agentConfigurationSchema.nullish(), note: z.string() }),
  health: z.object({ action: z.string(), text: z.string() }).nullish(),
});
export type OrgChannelPreview = z.infer<typeof orgChannelPreviewSchema>;
