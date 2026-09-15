import { z } from "zod";
import { connectMethodSchema } from "./connect-links";

export const appConnectItemSchema = z.object({
  requirement_id: z.string(),
  label: z.string(),
  optional: z.boolean(),
  state: z.enum([
    "unmet",
    "connecting",
    "reauthorizing",
    "validating",
    "met",
    "unknown",
    "failed",
    "skipped",
  ]),
  readiness: z.enum([
    "unmet",
    "met",
    "included",
    "unknown",
    "broken",
    "needs_reauth",
    "unsatisfiable",
    "disabled",
  ]),
  user_service_id: z.string().nullable(),
  slug: z.string().nullable(),
  resource_uri: z.string().nullable(),
  owner_id: z.string().nullable(),
  connect_link_id: z.string().nullable(),
  reason_code: z.string().nullable(),
  claim: z.string().nullable(),
  validated_at: z.string().nullable(),
  valid_until: z.string().nullable(),
  granted_to_caller: z.boolean(),
  catalog_slugs: z.array(z.string()),
  required_scopes: z.array(z.string()),
  choices: z.array(
    z.object({
      catalog_slug: z.string(),
      user_service_id: z.string(),
      slug: z.string(),
      owner_id: z.string(),
    }),
  ),
});
export const appConnectLinkSchema = z.object({
  id: z.string(),
  oauth_client_id: z.string(),
  client_name: z.string(),
  logo_url: z.string().nullable().optional(),
  verified: z.boolean().optional(),
  handoff_blurb: z.string().nullable(),
  destination: z.string(),
  requirements_version: z.number().int(),
  status: z.enum([
    "in_progress",
    "ready_for_consent",
    "completed",
    "cancelled",
    "expired",
    "failed",
  ]),
  expires_at: z.string(),
  items: z.array(appConnectItemSchema),
  callback_url: z.string().nullable(),
  consent_url: z.string().nullable().optional(),
  origin: z.enum(["app", "authorize"]).optional(),
  can_try_later: z.boolean().optional(),
  grant_update_required: z.boolean(),
});
export const appConnectChildSchema = z.object({
  id: z.string(),
  token: z.string(),
  service_name: z.string(),
  service_slug: z.string(),
  connect_method: connectMethodSchema,
  auth_key_name: z.string(),
  credential_mode: z.string().nullable(),
  has_platform_oauth_credentials: z.boolean(),
  requires_gateway_url: z.boolean(),
  api_key_url: z.string().nullable(),
  api_key_instructions: z.string().nullable(),
});
export const handoffFormSchema = z.object({
  handoff_blurb: z.string().trim().max(160, "Use at most 160 characters"),
});
export const handoffResponseSchema = z.object({
  handoff_blurb: z.string().nullable(),
});
export type AppConnectLink = z.infer<typeof appConnectLinkSchema>;
export type AppConnectItem = z.infer<typeof appConnectItemSchema>;
export type AppConnectChild = z.infer<typeof appConnectChildSchema>;
export type HandoffForm = z.infer<typeof handoffFormSchema>;
