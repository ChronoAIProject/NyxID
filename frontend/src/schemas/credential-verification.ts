import { z } from "zod";

export const credentialVerificationSchema = z.object({
  state: z.string(),
  reason_code: z.string(),
  observed_at: z.string().nullish(),
  source: z.string().nullish(),
  can_check: z.boolean().default(false),
});
export type CredentialVerification = z.infer<typeof credentialVerificationSchema>;
