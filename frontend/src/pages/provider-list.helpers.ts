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

function stripEmptyStrings<T extends Record<string, unknown>>(
  obj: T,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(obj).filter(
      ([, value]) => value !== "" && value !== undefined,
    ),
  );
}

export function buildCreateProviderPayload(
  data: CreateProviderFormData,
): Record<string, unknown> {
  const usesCredentialMode =
    data.provider_type === "oauth2" || data.provider_type === "device_code";

  return stripEmptyStrings({
    ...data,
    credential_mode: usesCredentialMode ? data.credential_mode : "admin",
    default_scopes: splitScopes(data.default_scopes),
    supports_pkce:
      data.provider_type === "oauth2" ? data.supports_pkce : undefined,
    client_id_param_name:
      data.provider_type === "telegram_widget"
        ? data.client_id_param_name
        : undefined,
  });
}

export function getProviderTypeFieldResets(
  previousType: ProviderType,
  nextType: ProviderType,
): Partial<
  Pick<CreateProviderFormData, "credential_mode" | "client_id_param_name">
> {
  const resets: Partial<
    Pick<CreateProviderFormData, "credential_mode" | "client_id_param_name">
  > = {};

  if (nextType === "telegram_widget") {
    resets.credential_mode = "admin";
  }

  if (previousType === "telegram_widget" && nextType !== "telegram_widget") {
    resets.client_id_param_name = "";
  }

  return resets;
}
