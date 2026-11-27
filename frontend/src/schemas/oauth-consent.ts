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

// Display only: the server verifies this signed request again on submission.
// Read every security-relevant label/selection from the same payload so changing
// URL hints cannot hide a required service or disguise the receiving app.
const incrementalConsentRequestSchema = z.object({
  exp: z.number(),
  token_type: z.literal("oauth_consent_request"),
  response_type: z.literal("code"),
  client_id: z.string().min(1),
  redirect_uri: z.string().url(),
  scope: z.string().min(1),
  state: z.string().nullable().optional(),
  nonce: z.string().nullish(),
  binding_grant_id: z.string().nullish(),
  external_subject_platform: z.string().nullish(),
  external_subject_tenant: z.string().nullish(),
  external_subject_external_user_id: z.string().nullish(),
  requested_service_ids: z.array(z.string()).default([]),
  code_challenge: z.string().min(1),
  code_challenge_method: z.literal("S256"),
  service_access_mode: z.literal("incremental"),
  resource: z.array(z.string()),
  incremental_consent: z.object({
    client_name: z.string().min(1),
    current_scopes: z.string(),
    scopes: z.string().min(1),
    current_service_ids: z.array(z.string().min(1)),
    allow_all_services: z.boolean(),
    required_service_ids: z.array(z.string().min(1)),
  }),
});

export type IncrementalConsentRequest = z.infer<
  typeof incrementalConsentRequestSchema
> & { readonly token: string };

export function readIncrementalConsentRequest(
  search: URLSearchParams,
): { request: IncrementalConsentRequest } | { error: string } | null {
  const token = search.get("consent_request") ?? "";
  try {
    const part = token.split(".")[1] ?? "";
    const base64 = part.replace(/-/g, "+").replace(/_/g, "/");
    const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
    const payload: unknown = JSON.parse(new TextDecoder().decode(bytes));
    if (
      typeof payload === "object" &&
      payload !== null &&
      "service_access_mode" in payload &&
      payload.service_access_mode === "incremental"
    ) {
      const result = incrementalConsentRequestSchema.safeParse(payload);
      return result.success
        ? { request: { ...result.data, token } }
        : { error: "Invalid consent request. Please restart authorization." };
    }
  } catch {
    // Existing consent requests are handled by the ordinary page. An explicit
    // incremental request must never fall back to editable replacement mode.
  }
  return search.get("service_access_mode") === "incremental"
    ? { error: "Invalid consent request. Please restart authorization." }
    : null;
}

export const MAX_CONSENT_DISCLOSURE_SERVICES = 100;

const upstreamScopeSchema = z.discriminatedUnion("known", [
  z.object({
    scope: z.string().min(1),
    known: z.literal(true),
    label: z.string().min(1),
    description: z.string().min(1),
    sensitive: z.boolean(),
  }),
  z.object({
    scope: z.string().min(1),
    known: z.literal(false),
    label: z.null(),
    description: z.null(),
    sensitive: z.null(),
  }),
]);

const serviceDisclosureSchema = z.object({
  service_id: z.string().min(1),
  credential_source: z.discriminatedUnion("type", [
    z.object({ type: z.literal("personal") }),
    z.object({ type: z.literal("org"), org_name: z.string() }),
  ]),
  credential_kind: z.enum(["oauth", "api_key", "platform", "node_managed", "other", "unknown"]),
  upstream_scopes: z.array(upstreamScopeSchema).nullable(),
  known_operations: z.object({
    read: z.int().nonnegative(),
    write: z.int().nonnegative(),
    delete: z.int().nonnegative(),
    unclassified: z.int().nonnegative(),
    limiting: z.literal(false),
  }).nullable(),
  capabilities: z.object({
    model_calls: z.literal(true).nullable(),
    binary_upload: z.literal(true).nullable(),
  }),
});

export const consentDisclosureSchema = z.object({
  access_token_ttl_secs: z.int().positive(),
  broker_access_token_ttl_secs: z.int().positive().optional(),
  refresh_token_ttl_secs: z.int().positive(),
  services: z.array(z.unknown()).max(MAX_CONSENT_DISCLOSURE_SERVICES).transform((rows) =>
    rows.flatMap((row) => {
      const result = serviceDisclosureSchema.safeParse(row);
      return result.success ? [result.data] : [];
    }),
  ),
});

export type ServiceConsentDisclosure = z.infer<typeof serviceDisclosureSchema>;
export type ConsentDisclosure = z.infer<typeof consentDisclosureSchema>;

export const existingConsentAccessSchema = z.object({
  allow_all_services: z.boolean(),
  allowed_service_ids: z.array(z.string()),
  expires_at: z.iso.datetime({ offset: true }).nullable(),
});
