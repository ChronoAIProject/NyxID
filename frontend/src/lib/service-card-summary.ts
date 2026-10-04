import type {
  ServiceBillingExplanation,
  ConfiguredCatalogEntry,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";
import {
  configuredUsageCharge,
  connectionCredentialClass,
  positiveUsageRate,
} from "./service-billing-config";

export type ConnectionBillingCategory =
  | "platform"
  | "byok"
  | "not_billable"
  | "unknown";

export const connectionBillingLabels: Record<
  ConnectionBillingCategory,
  string
> = {
  platform: "NyxID credentials",
  byok: "BYOK",
  not_billable: "—",
  unknown: "Unverified",
};

export function connectionBillingCategory(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
  catalog?: ConfiguredCatalogEntry,
): ConnectionBillingCategory {
  if (billing?.status === "restricted") return "unknown";
  const notBillable =
    connectionBillability(connection, billing, catalog) === false;
  // A resolved caller override takes precedence over the connection default.
  switch (connectionCredentialClass(connection, billing)) {
    case "nyxid_managed_master":
    case "nyxid_platform_oauth_app":
      return notBillable ? "not_billable" : "platform";
    case "user_owned":
    case "agent_override_user_owned":
    case "node_managed":
      return "byok";
    case "no_auth":
      return notBillable ? "not_billable" : "unknown";
  }
  if (billing?.context === "agent_key")
    return notBillable ? "not_billable" : "unknown";
  if (
    connection.credential_binding === "platform" ||
    billing?.provider_billing === "nyxid_credential"
  )
    return notBillable ? "not_billable" : "platform";
  // Credential provenance can be unknown without billing being unknown.
  return notBillable ? "not_billable" : "unknown";
}

export function connectionBillability(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
  catalog?: ConfiguredCatalogEntry,
): boolean | undefined {
  if (billing?.status === "restricted") return undefined;
  if (billing?.credit_billing_configured != null)
    return billing.credit_billing_configured;
  if (billing?.context === "agent_key" && !billing.credential_class)
    return undefined;
  const configured = configuredUsageCharge(
    connection,
    catalog,
    billing?.credential_class,
  );
  if (configured === true) return true;
  if (billing?.status === "unavailable") return configured;
  if (billing?.rates.length)
    return billing.rates.some(
      (rate) =>
        rate.credits_per_unit == null ||
        positiveUsageRate(rate.credits_per_unit),
    );
  if (billing?.charge_status === "usage_based") return true;
  // "Not charged" can describe caller rollout rather than the service's configuration.
  return configured;
}

export function latestServiceEdit(connections: readonly KeyInfo[]) {
  return connections
    .flatMap((connection) => {
      const edit = connection.authorship?.last_change;
      return edit && Number.isFinite(Date.parse(edit.at))
        ? [{ connection, edit }]
        : [];
    })
    .sort((a, b) => Date.parse(b.edit.at) - Date.parse(a.edit.at))[0];
}
