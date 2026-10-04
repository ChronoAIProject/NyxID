import type {
  ConfiguredCatalogEntry,
  ServiceBillingExplanation,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";

/** Catalog pricing describes the service's platform offering, not the selected credential. */
export function configuredPlatformPrice(
  connection: KeyInfo,
  catalog?: ConfiguredCatalogEntry,
) {
  return (
    connection.platform_key_pricing ??
    catalog?.platform_key?.pricing ??
    catalog?.billing?.platform_key_pricing
  );
}

export function positiveUsageRate(rate: string): boolean {
  return /^\d+(?:\.\d+)?$/.test(rate) && /[1-9]/.test(rate);
}

/** Connection metadata identifies credential supply; catalog prices never do. */
export function connectionCredentialClass(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
) {
  if (billing?.status === "restricted") return undefined;
  if (billing?.credential_class) return billing.credential_class;
  if (billing?.context === "agent_key") return undefined;
  if (connection.credential_binding === "platform")
    return "nyxid_managed_master";
  if (connection.node_id || connection.has_node_binding) return undefined;
  if (connection.auth_method === "none") return "no_auth";
  if (connection.credential_missing) return undefined;
  if (
    ["oauth2", "device_code"].includes(connection.credential_type) &&
    connection.oauth_client_id?.trim()
  )
    return "user_owned";
  if (
    connection.credential_type === "api_key" &&
    (connection.api_key_id || connection.credential_binding === "user")
  )
    return "user_owned";
  return undefined;
}

/** Configured charges, independent of connection status, grants or wallet balance. */
export function configuredUsageCharge(
  connection: KeyInfo,
  catalog?: ConfiguredCatalogEntry,
  credentialClass?: string | null,
): boolean | undefined {
  credentialClass ??= connectionCredentialClass(connection);
  const platform = credentialClass
    ? credentialClass === "nyxid_managed_master"
    : connection.credential_binding === "platform";
  const noAuth = credentialClass
    ? credentialClass === "no_auth"
    : connection.auth_method === "none" &&
      !connection.node_id &&
      !connection.has_node_binding;
  const oauthUnknown =
    !credentialClass &&
    !platform &&
    ["oauth2", "device_code"].includes(connection.credential_type);
  const billing = catalog?.billing;
  const byok =
    connection.byok_pricing ?? catalog?.byok_pricing ?? billing?.byok_pricing;
  const pk = configuredPlatformPrice(connection, catalog);
  const lane = noAuth ? undefined : platform ? pk : byok;
  const legacy =
    Boolean(billing?.platform_billable) &&
    (!billing?.platform_pricing ||
      billing.platform_pricing.sync_status !== "synced" ||
      positiveUsageRate(billing.platform_pricing.credits_per_unit));
  const platformCharge =
    byok || pk
      ? Boolean(
          lane &&
          ([lane, ...(lane.components ?? [])].some((rate) =>
            positiveUsageRate(rate.credits_per_unit),
          ) ||
            (lane.sync_status !== "synced" && legacy)),
        )
      : legacy;
  if (platform && billing?.resale_billable) return true;
  if (
    billing?.platform_charge_nyxid_credentials_only &&
    !platform &&
    credentialClass !== "nyxid_platform_oauth_app"
  ) {
    return oauthUnknown && platformCharge ? undefined : false;
  }
  if (platformCharge) return true;
  // The catalog omits `billing` when no configuration exists. A missing
  // catalog entry is different: a failed or restricted read proves nothing.
  const custom =
    !connection.catalog_service_id &&
    !connection.catalog_service_slug &&
    (connection.source === "custom" ||
      (connection.catalog_service_id === null &&
        connection.catalog_service_slug === null));
  if (byok || pk || catalog || custom) return false;
  return undefined;
}
