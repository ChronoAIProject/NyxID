import { z } from "zod";
export const guestServiceSchema = z.object({
  id: z.string(),
  label: z.string(),
  access: z.enum(["none", "read", "use", "all"]),
  limited_operations: z.boolean(),
});
export const orgGuestSettingsSchema = z.object({
  channel_id: z.string(),
  org_id: z.string(),
  agent_id: z.string().nullish(),
  can_manage: z.boolean(),
  guest_revision: z.string().default(""),
  guest_enabled: z.boolean(),
  grants_ready: z.boolean().default(false),
  guest_policy_mode: z.enum(["legacy", "grants"]).default("legacy"),
  channel_status: z.string().optional(),
  readiness_reason: z.string().nullish(),
  effective_guest_services: z.array(guestServiceSchema).default([]),
  spent_today_credits: z.string(),
  reserved_today_credits: z.string(),
});
export const orgGuestReviewSchema = z.object({
  ready: z.boolean(),
  pending: z.boolean(),
  review: z.string(),
  bots: z.array(
    z.object({ id: z.string(), name: z.string(), status: z.string() }),
  ),
  services: z.array(guestServiceSchema),
});
export function orgGuestSwitchPayload(
  enabled: boolean,
  expected_revision: string,
) {
  return { enabled, expected_revision };
}
export const GUEST_ACCESS_LABEL = {
  none: "No guest access",
  read: "Look things up only",
  use: "Use, but not change or delete",
  all: "Everything this agent can do",
};
