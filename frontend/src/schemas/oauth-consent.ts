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

export const consentPresentationSchema = z.object({
  client_id: z.string(),
  client_name: z.string(),
  redirect_uri: z.string(),
  scope: z.string(),
  resources: z.array(z.string()),
  mandatory_service_ids: z.array(z.string()),
  selectable_service_ids: z.array(z.string()),
  app_connect_link_id: z.string().nullable(),
});
export type ConsentPresentation = z.infer<typeof consentPresentationSchema>;
