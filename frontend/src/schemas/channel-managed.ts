import { z } from "zod";

const metaId = z.string().regex(/^\d{1,32}$/);
export const managedBootstrapSchema = z.object({
  available: z.boolean(),
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
export const managedCompleteSchema = z.object({
  code: z.string().min(1).max(8192),
  phone_number_id: metaId.optional(),
  waba_id: metaId,
  business_id: metaId.optional(),
  label: z.string().trim().min(1).max(128),
  target_org_id: z.string().optional(),
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
