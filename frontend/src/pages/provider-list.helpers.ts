import type { CreateProviderFormData } from "@/schemas/providers";

type ProviderType = CreateProviderFormData["provider_type"];

export function splitScopes(
  raw: string | undefined,
): readonly string[] | undefined {
  if (!raw || raw.trim() === "") return undefined;
  return raw
    .split(",")
    .map((scope) => scope.trim())
    .filter((scope) => scope.length > 0);
}

/** Fields shared by every provider type. */
const COMMON_FIELDS: ReadonlySet<string> = new Set([
  "name",
  "slug",
  "description",
  "provider_type",
  "icon_url",
  "documentation_url",
]);

/** Fields relevant to each provider type (beyond COMMON_FIELDS). */
const TYPE_FIELDS: Record<ProviderType, ReadonlySet<string>> = {
  oauth2: new Set([
    "authorization_url",
    "token_url",
    "revocation_url",
    "default_scopes",
    "client_id",
    "client_secret",
    "supports_pkce",
    "credential_mode",
    "token_endpoint_auth_method",
    "extra_auth_params",
    "hosted_callback_url",
  ]),
  device_code: new Set([
    "authorization_url",
    "token_url",
    "default_scopes",
    "client_id",
    "client_secret",
    "credential_mode",
    "device_code_url",
    "device_token_url",
    "device_verification_url",
    "device_code_format",
    "token_endpoint_auth_method",
  ]),
  api_key: new Set(["api_key_instructions", "api_key_url"]),
  telegram_widget: new Set(["client_id_param_name", "client_secret"]),
};

export function buildCreateProviderPayload(
  data: CreateProviderFormData,
): Record<string, unknown> {
  const allowed = TYPE_FIELDS[data.provider_type];
  const result: Record<string, unknown> = {};

  for (const [key, value] of Object.entries(data)) {
    if (!COMMON_FIELDS.has(key) && !allowed.has(key)) continue;
    if (value === "" || value === undefined) continue;
    result[key] = value;
  }

  // Normalize credential_mode for types that don't expose it.
  if (data.provider_type !== "oauth2" && data.provider_type !== "device_code") {
    result.credential_mode = "admin";
  }

  // Normalize scopes to array.
  if (data.default_scopes) {
    result.default_scopes = splitScopes(data.default_scopes);
  }

  // supports_pkce only applies to oauth2.
  if (data.provider_type !== "oauth2") {
    delete result.supports_pkce;
  }

  return result;
}

/** All form fields that are type-specific (union of all TYPE_FIELDS values). */
type ResettableField = keyof Pick<
  CreateProviderFormData,
  | "credential_mode"
  | "client_id_param_name"
  | "authorization_url"
  | "token_url"
  | "revocation_url"
  | "default_scopes"
  | "client_id"
  | "client_secret"
  | "supports_pkce"
  | "device_code_url"
  | "device_token_url"
  | "device_verification_url"
  | "api_key_instructions"
  | "api_key_url"
>;

const RESETTABLE_DEFAULTS: Record<ResettableField, string | boolean | undefined> = {
  credential_mode: undefined,
  client_id_param_name: "",
  authorization_url: "",
  token_url: "",
  revocation_url: "",
  default_scopes: "",
  client_id: "",
  client_secret: "",
  supports_pkce: false,
  device_code_url: "",
  device_token_url: "",
  device_verification_url: "",
  api_key_instructions: "",
  api_key_url: "",
};

export function getProviderTypeFieldResets(
  previousType: ProviderType,
  nextType: ProviderType,
): Partial<Record<ResettableField, string | boolean | undefined>> {
  if (previousType === nextType) return {};

  const prevFields = TYPE_FIELDS[previousType];
  const nextFields = TYPE_FIELDS[nextType];

  const resets: Partial<Record<ResettableField, string | boolean | undefined>> = {};

  // Clear fields that belonged to the previous type but not the next.
  for (const field of prevFields) {
    if (!nextFields.has(field) && field in RESETTABLE_DEFAULTS) {
      resets[field as ResettableField] =
        RESETTABLE_DEFAULTS[field as ResettableField];
    }
  }

  // Force credential_mode to admin for telegram_widget.
  if (nextType === "telegram_widget") {
    resets.credential_mode = "admin";
  }

  return resets;
}
