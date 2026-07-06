import { z } from "zod";

export const oauthConsentServiceAccessSchema = z
  .object({
    allow_all_services: z.boolean(),
    allowed_service_ids: z.array(z.string().trim().min(1)),
  })
  .transform((value) => ({
    allow_all_services: value.allow_all_services,
    allowed_service_ids: value.allow_all_services
      ? []
      : Array.from(new Set(value.allowed_service_ids)),
  }));

export type OAuthConsentServiceAccess = z.infer<
  typeof oauthConsentServiceAccessSchema
>;
