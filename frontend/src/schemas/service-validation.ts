import { z } from "zod";

export const serviceValidationOutcomeSchema = z.enum([
  "authenticated",
  "permission_denied",
  "credential_rejected",
  "configuration_error",
  "billing_blocked",
  "rate_limited",
  "transport_unknown",
  "unsupported",
]);

export const serviceValidationResponseSchema = z.object({
  user_service_id: z.string(),
  validator_id: z.string(),
  validator_version: z.number().int().nonnegative(),
  outcome: serviceValidationOutcomeSchema,
  claim: z.string(),
  checked_at: z.iso.datetime({ offset: true }),
  valid_until: z.iso.datetime({ offset: true }),
  reason_code: z.string(),
});

export type ServiceValidationResponse = z.infer<
  typeof serviceValidationResponseSchema
>;
