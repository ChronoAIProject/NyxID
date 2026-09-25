import { z } from "zod";

export const approvalIdentitySchema = z.object({
  id: z.string().uuid(),
  flow: z.enum(["device", "agent-key"]),
  user_code: z.string(),
  verified: z.boolean(),
  mfa_required: z.boolean(),
  keep_signed_in: z.boolean(),
  expires_at: z.string().datetime({ offset: true }),
  user: z
    .object({
      id: z.string(),
      email: z.string(),
      display_name: z.string().nullable(),
    })
    .nullable(),
});
export type ApprovalIdentity = z.infer<typeof approvalIdentitySchema>;

export const approvalMfaSchema = z.object({
  code: z.string().regex(/^\d{6}$/, "Enter the six-digit authenticator code"),
});
