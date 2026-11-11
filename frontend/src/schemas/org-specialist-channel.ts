import { z } from "zod";

export const orgChannelSettingsSchema = z.object({
  channel_id: z.string(),
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
export const orgChannelGuestFormSchema = z.object({
  enabled: z.boolean(),
  service_ids: z.array(z.string()),
  daily_credits: z
    .string()
    .regex(
      /^\d+(?:\.\d{1,12})?$/,
      "Enter a positive credit amount, with up to 12 decimal places",
    )
    .refine((v) => /[1-9]/.test(v), "The daily budget must be positive"),
});
export type OrgChannelGuestForm = z.infer<typeof orgChannelGuestFormSchema>;
export type OrgChannelGuestUpdate = OrgChannelGuestForm & { expected_revision?: string };
export const orgChannelChallengeSchema = z.object({
  id: z.string(),
  state: z.string(),
  code: z.string().optional(),
  expires_at: z.string().optional(),
  sender_label: z.string().nullish(),
});
