import { z } from "zod";

export const orgChannelSettingsSchema = z.object({
  channel_id: z.string(),
  agent_id: z.string().nullish(),
  grants_ready: z.boolean().default(false),
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
  daily_credits: z.string().nullable(),
  default_daily_credits: z.string(),
  spent_today_credits: z.string(),
  reserved_today_credits: z.string(),
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
