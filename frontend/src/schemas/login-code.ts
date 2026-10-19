import { z } from "zod";

export const loginCodeSchema = z.object({
  request_id: z.string(), code: z.string(), expires_at: z.iso.datetime({ offset: true }),
});
export const loginCodeStatusSchema = z.object({
  request_id: z.string(), status: z.enum(["pending", "redeemed", "expired", "cancelled", "revoked"]),
  auth_kind: z.enum(["agent_key", "account_session"]), expires_at: z.string(),
  redeemed_at: z.string().nullable(), client_label: z.string().nullable(),
  client_user_agent: z.string().nullable(), client_ip: z.string().nullable(),
  client_ip_attribution: z.string(), can_revoke: z.boolean(),
});
export type LoginCode = z.infer<typeof loginCodeSchema>;
