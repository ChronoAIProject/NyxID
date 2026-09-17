import type { ProviderConfig } from "@/types/api";
import type { UpdateProviderFormData } from "@/schemas/providers";
// `splitScopes` is functionally identical to the implementation in
// provider-list.helpers.ts, so it is re-exported here rather than duplicated.
import { splitScopes } from "./provider-list.helpers";
export { splitScopes };

export const PROVIDER_TYPE_LABELS: Readonly<Record<string, string>> = {
  oauth2: "OAuth 2.0",
  api_key: "API Key",
  device_code: "Device Code",
  telegram_widget: "Telegram Widget",
};

export function stripEmptyStrings<T extends Record<string, unknown>>(
  obj: T,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(obj).filter(([, v]) => v !== "" && v !== undefined),
  );
}

export function providerFormValues(
  provider: ProviderConfig,
): UpdateProviderFormData {
  return {
    name: provider.name,
    slug: provider.slug,
    description: provider.description ?? "",
    provider_type: provider.provider_type,
    credential_mode: provider.credential_mode ?? "admin",
    authorization_url: provider.authorization_url ?? "",
    token_url: provider.token_url ?? "",
    revocation_url: provider.revocation?.url ?? provider.revocation_url ?? "",
    default_scopes: provider.default_scopes?.join(", ") ?? "",
    is_active: provider.is_active,
    client_id: "",
    client_secret: "",
    client_id_param_name: provider.client_id_param_name ?? "",
    supports_pkce: provider.supports_pkce,
    device_code_url: provider.device_code_url ?? "",
    device_token_url: provider.device_token_url ?? "",
    hosted_callback_url: provider.hosted_callback_url ?? "",
    api_key_instructions: provider.api_key_instructions ?? "",
    api_key_url: provider.api_key_url ?? "",
    icon_url: provider.icon_url ?? "",
    documentation_url: provider.documentation_url ?? "",
  };
}

export function providerFormPayload(data: UpdateProviderFormData) {
  const { slug: _slug, provider_type: _type, ...fields } = data;
  void _slug;
  const { client_id, client_secret, ...settings } = fields;
  return {
    ...settings,
    default_scopes: splitScopes(data.default_scopes) ?? [],
    supports_pkce: _type === "oauth2" ? data.supports_pkce : undefined,
    credential_mode:
      _type === "oauth2" || _type === "device_code"
        ? data.credential_mode
        : undefined,
    ...(client_id?.trim() ? { client_id: client_id.trim() } : {}),
    ...(client_secret?.trim() ? { client_secret: client_secret.trim() } : {}),
  };
}
