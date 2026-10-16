import { z } from "zod";

const credentialSourceSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("personal") }),
  z.object({
    type: z.literal("org"),
    org_id: z.string(),
    org_name: z.string(),
    role: z.enum(["owner", "admin", "member", "viewer"]),
    allowed: z.boolean(),
  }),
]);

export const serviceConnectionSchema = z.object({
  id: z.string(),
  label: z.string(),
  slug: z.string(),
  description: z.string().nullish(),
  status: z.string(),
  is_active: z.boolean(),
  auto_connected: z.boolean(),
  can_edit_configuration: z.boolean().optional(),
  credential_source: credentialSourceSchema.optional(),
  credential_type: z.string(),
  service_type: z.string().optional(),
  endpoint_url: z.string().nullish(),
  api_key_id: z.string().nullish(),
  credential_missing: z.boolean().optional(),
  credential_binding: z.enum(["platform", "user"]).optional(),
  catalog_service_slug: z.string().nullish(),
  catalog_service_name: z.string().nullish(),
  connection_status: z.enum(["active", "expired"]).nullish(),
  expires_at: z.string().nullish(),
  last_authorized_at: z.string().nullish(),
  last_used_at: z.string().nullish(),
  granted_scopes: z.array(z.string()).nullish(),
  error_message: z.string().nullish(),
  proxy_url: z.string().optional(),
  docs_url: z.string().nullish(),
  revocation: z.object({ revokes_grant: z.boolean() }).nullish(),
  offering_kind: z.enum(["ai_service", "tool"]).optional(),
});
export type ServiceConnection = z.infer<typeof serviceConnectionSchema>;

export const catalogEntrySchema = z.object({
  slug: z.string(),
  name: z.string(),
  description: z.string().nullish(),
  base_url: z.string().optional(),
  auth_method: z.string(),
  auth_key_name: z.string(),
  service_type: z.string(),
  provider_config_id: z.string().nullish(),
  provider_type: z.string().nullish(),
  supports_oauth_scopes: z.boolean().optional(),
  device_code_format: z.string().nullish(),
  revocation: z.object({ revokes_grant: z.boolean() }).nullish(),
  scope_catalog: z
    .array(
      z.object({
        scope: z.string(),
        label: z.string(),
        description: z.string().nullish(),
        required: z.boolean().optional(),
      }),
    )
    .nullish(),
  credential_mode: z.string().nullish(),
  has_platform_oauth_credentials: z.boolean().optional(),
  requires_credential: z.boolean(),
  requires_gateway_url: z.boolean(),
  api_key_instructions: z.string().nullish(),
  api_key_url: z.string().nullish(),
  documentation_url: z.string().nullish(),
  platform_key: z.object({ available: z.boolean() }).optional(),
  token_exchange_credential_fields: z
    .array(
      z.object({
        name: z.string(),
        label: z.string(),
        placeholder: z.string().nullish(),
        secret: z.boolean(),
      }),
    )
    .nullish(),
});
export type CatalogEntry = z.infer<typeof catalogEntrySchema>;

export function canEditConnection(connection: ServiceConnection): boolean {
  if (connection.auto_connected || connection.can_edit_configuration === false)
    return false;
  if (connection.can_edit_configuration === true) return true;
  const source = connection.credential_source;
  return (
    source?.type === "personal" ||
    (source?.type === "org" && source.role === "admin" && source.allowed)
  );
}

export function connectionOwner(connection: ServiceConnection): string {
  return connection.credential_source?.type === "org"
    ? connection.credential_source.org_name
    : "Personal";
}

/** Match the canonical catalog slug, never the suffixed connection proxy slug. */
export function catalogForConnection(
  connection: ServiceConnection,
  entries: CatalogEntry[],
) {
  return entries.find(
    (entry) => entry.slug === connection.catalog_service_slug,
  );
}

export function supportsManagedOAuth(entry: CatalogEntry): boolean {
  return (
    entry.provider_type === "oauth2" &&
    Boolean(entry.provider_config_id) &&
    entry.credential_mode !== "user" &&
    (entry.credential_mode !== "both" ||
      entry.has_platform_oauth_credentials === true)
  );
}

export function supportsCredentialForm(entry: CatalogEntry): boolean {
  return (
    entry.service_type !== "ssh" &&
    !["oauth2", "device_code", "telegram"].includes(
      entry.provider_type ?? "",
    ) &&
    !["aws_sigv4", "ifttt_webhook", "ifttt_mcp"].includes(entry.auth_method)
  );
}

export function validateBrowserUrl(value: string): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error("The service returned an invalid browser link.");
  }
  if (url.protocol !== "https:" || url.username || url.password) {
    throw new Error("The service must return a secure HTTPS browser link.");
  }
  return url.toString();
}

export function buildCatalogConnectionInput(
  entry: CatalogEntry,
  values: {
    label: string;
    credential: string;
    endpointUrl: string;
    fields: Record<string, string>;
    platform: boolean;
  },
) {
  const label = values.label.trim();
  if (!label) throw new Error("Give this connection a name.");
  if (values.platform) {
    if (!entry.platform_key?.available)
      throw new Error("A platform key is not available for this service.");
    return { service_slug: entry.slug, label, use_platform_key: true };
  }
  let endpoint_url: string | undefined;
  if (entry.requires_gateway_url) {
    endpoint_url = validateBrowserUrl(values.endpointUrl.trim());
  }
  let credential = values.credential.trim();
  if (entry.token_exchange_credential_fields?.length) {
    const fields: Record<string, string> = {};
    for (const field of entry.token_exchange_credential_fields) {
      const value = values.fields[field.name]?.trim();
      if (!value) throw new Error(`Enter ${field.label}.`);
      fields[field.name] = value;
    }
    credential = JSON.stringify(fields);
  }
  if (
    entry.requires_credential &&
    !credential &&
    !supportsManagedOAuth(entry)
  ) {
    throw new Error("Enter the service credential.");
  }
  return {
    service_slug: entry.slug,
    label,
    ...(endpoint_url ? { endpoint_url } : {}),
    ...(credential ? { credential } : {}),
  };
}
