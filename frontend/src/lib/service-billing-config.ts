import type {
  ConfiguredCatalogEntry,
  ServiceBillingExplanation,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";

export type CredentialSupplier = "nyxid" | "own" | "none" | "unknown";

/** Catalog pricing describes the service offering, not the selected credential. */
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

export function credentialSupplier(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
): CredentialSupplier {
  if (billing?.status === "restricted") return "unknown";
  if (billing?.credential_supplier != null) return billing.credential_supplier;
  if (
    ["nyxid_managed_master", "nyxid_platform_oauth_app"].includes(
      billing?.credential_class ?? "",
    )
  )
    return "nyxid";
  // A selected agent can use an entirely different key from this connection.
  if (billing?.context === "agent_key") return "unknown";
  if (
    billing?.credential_class === "user_owned" &&
    !["oauth2", "device_code"].includes(connection.credential_type)
  )
    return "own";
  if (connection.credential_binding === "platform") return "nyxid";
  const node = connection.node_id || connection.has_node_binding;
  if (connection.auth_method === "none" && !node) return "none";
  if (connection.credential_missing) return "unknown";
  if (["oauth2", "device_code"].includes(connection.credential_type)) {
    if (connection.oauth_app_source === "platform") return "nyxid";
    if (
      connection.oauth_app_source === "byo" ||
      connection.oauth_client_id?.trim()
    )
      return "own";
    // Legacy execution class UserOwned is a price-lane default, not app provenance.
    return "unknown";
  }
  if (node && connection.credential_type === "node_managed") return "own";
  if (
    connection.api_key_id &&
    [
      "api_key",
      "bearer",
      "basic",
      "token_exchange",
      "ssh_certificate",
    ].includes(connection.credential_type)
  )
    return "own";
  return "unknown";
}

/** Execution class for rates. Supplier and price lane are distinct for OAuth. */
export function connectionCredentialClass(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
) {
  if (billing?.status === "restricted") return undefined;
  if (billing?.credential_class) return billing.credential_class;
  if (billing?.context === "agent_key") return undefined;
  const supplier = credentialSupplier(connection);
  if (connection.credential_binding === "platform")
    return "nyxid_managed_master";
  if (supplier === "none") return "no_auth";
  if (supplier === "nyxid") return "nyxid_platform_oauth_app";
  if (supplier === "own")
    return connection.credential_type === "node_managed"
      ? "node_managed"
      : "user_owned";
  return undefined;
}

function configuration(connection: KeyInfo, catalog?: ConfiguredCatalogEntry) {
  return {
    ...catalog?.billing,
    byok_pricing:
      connection.byok_pricing ??
      catalog?.byok_pricing ??
      catalog?.billing?.byok_pricing,
    platform_key_pricing: configuredPlatformPrice(connection, catalog),
  };
}

/** Mirrors the backend's configured_usage_charge; no connection inference here. */
export function configuredChargeForClass(
  billing: NonNullable<ConfiguredCatalogEntry["billing"]>,
  credentialClass: string,
): boolean {
  const master = credentialClass === "nyxid_managed_master";
  if (master && billing.resale_billable) return true;
  if (
    billing.platform_charge_nyxid_credentials_only &&
    !master &&
    credentialClass !== "nyxid_platform_oauth_app"
  )
    return false;
  const legacy =
    Boolean(billing.platform_billable) &&
    (!billing.platform_pricing ||
      billing.platform_pricing.sync_status !== "synced" ||
      positiveUsageRate(billing.platform_pricing.credits_per_unit));
  if (!billing.byok_pricing && !billing.platform_key_pricing) return legacy;
  const lane =
    credentialClass === "no_auth"
      ? undefined
      : master
        ? billing.platform_key_pricing
        : billing.byok_pricing;
  return Boolean(
    lane &&
    ([lane, ...(lane.components ?? [])].some((rate) =>
      positiveUsageRate(rate.credits_per_unit),
    ) ||
      (lane.sync_status !== "synced" && legacy)),
  );
}

/** The service-wide gate must be checked before credential supply. */
export function serviceBillingConfigured(
  connection: KeyInfo,
  catalog?: ConfiguredCatalogEntry,
): boolean | undefined {
  const billing = configuration(connection, catalog);
  if (
    [
      "nyxid_managed_master",
      "nyxid_platform_oauth_app",
      "user_owned",
      "no_auth",
    ].some((cls) => configuredChargeForClass(billing, cls))
  )
    return true;
  // KeyResponse omits null catalog ids/slugs for custom services.
  if (
    catalog ||
    (!connection.catalog_service_id && !connection.catalog_service_slug)
  )
    return false;
  return undefined;
}

/** Charge for this connection, never the service-wide label gate. */
export function configuredUsageCharge(
  connection: KeyInfo,
  catalog?: ConfiguredCatalogEntry,
  credentialClass?: string | null,
): boolean | undefined {
  if (serviceBillingConfigured(connection, catalog) === false) return false;
  const billing = configuration(connection, catalog);
  const hasConfiguration = Boolean(
    catalog || billing.byok_pricing || billing.platform_key_pricing,
  );
  if (!hasConfiguration) return undefined;
  credentialClass ??= connectionCredentialClass(connection);
  const classes = credentialClass
    ? [credentialClass]
    : ["oauth2", "device_code"].includes(connection.credential_type)
      ? ["nyxid_platform_oauth_app", "user_owned"]
      : ["nyxid_managed_master", "user_owned", "node_managed"];
  const charges = classes.map((cls) => configuredChargeForClass(billing, cls));
  return charges.every((value) => value === charges[0])
    ? charges[0]
    : undefined;
}
