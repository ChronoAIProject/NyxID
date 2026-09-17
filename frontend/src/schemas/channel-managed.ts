import { z } from "zod";

const metaId = z.string().regex(/^\d{1,32}$/);
export const managedBootstrapSchema = z.object({
  available: z.boolean(),
  flow: z.enum(["meta_embedded_signup", "oauth_connection"]).nullable().optional(),
  provider_slug: z.string().nullable().optional(),
  required_scopes: z.array(z.string()).default([]),
  authorize_start_url: z.string().nullable().optional(),
  app_id: metaId.optional(),
  embedded_signup_config_id: metaId.optional(),
  graph_version: z.string().nullable().optional(),
  signup_version: z.string().nullable().optional(),
  signup_extras: z
    .record(z.string(), z.record(z.string(), z.unknown()))
    .default({}),
  feature_types: z.array(z.string()).default([]),
});
export type ManagedBootstrap = z.infer<typeof managedBootstrapSchema>;
const embeddedSignupCompleteSchema = z.object({
  code: z.string().min(1).max(8192),
  phone_number_id: metaId.optional(),
  waba_id: metaId,
  business_id: metaId.optional(),
  label: z.string().trim().min(1).max(128),
  target_org_id: z.string().optional(),
});
export const oauthConnectionCompleteSchema = z.object({
  connection_id: z.uuid(), label: z.string().trim().min(1).max(128), target_org_id: z.string().optional(),
}).strict();
export const managedCompleteSchema = z.union([embeddedSignupCompleteSchema, oauthConnectionCompleteSchema]);
export const managedOAuthStartSchema = z.object({
  connection_id: z.uuid(), authorization_url: z.url(), attempt_nonce: z.uuid(),
});
export type ManagedCompleteInput = z.infer<typeof managedCompleteSchema>;
export const embeddedSignupEventSchema = z.object({
  type: z.literal("WA_EMBEDDED_SIGNUP"),
  event: z.enum([
    "FINISH",
    "FINISH_ONLY_WABA",
    "FINISH_WHATSAPP_BUSINESS_APP_ONBOARDING",
    "FINISH_OBO_MIGRATION",
    "FINISH_GRANT_ONLY_API_ACCESS",
    "CANCEL",
    "ERROR",
  ]),
  data: z
    .object({
      phone_number_id: metaId.optional(),
      waba_id: metaId.optional(),
      waba_ids: z.array(metaId).max(100).optional(),
      business_id: metaId.optional(),
      current_step: z.string().max(100).optional(),
      error_message: z.string().max(4096).optional(),
      error_code: z.string().max(100).optional(),
      session_id: z.string().max(256).optional(),
      timestamp: z.string().max(32).optional(),
    })
    .default({}),
});
