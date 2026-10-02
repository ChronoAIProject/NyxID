import type { ConfiguredCatalogEntry } from "@/schemas/service-insights";
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

/** Configured charges, independent of connection status, grants or wallet balance. */
export function configuredUsageCharge(
  connection: KeyInfo,
  catalog?: ConfiguredCatalogEntry,
  credentialClass?: string | null,
): boolean | undefined {
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
  // Omitted catalog billing on an older server is not a declaration of free usage.
  if (byok || pk || (catalog && Object.hasOwn(catalog, "billing")))
    return false;
  return undefined;
}
