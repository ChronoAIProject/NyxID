import { z } from "zod";

export const securityPreferencesSchema = z.object({
  skip_mfa_for_approved_sign_ins: z.boolean(),
}).strict();
