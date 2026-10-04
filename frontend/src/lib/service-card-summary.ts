import type {
  ServiceBillingExplanation,
  ConfiguredCatalogEntry,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";
import {
  configuredUsageCharge,
  credentialSupplier,
  serviceBillingConfigured,
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
  platform: "NyxID",
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
  const serviceConfigured =
    billing?.service_billing_configured ??
    serviceBillingConfigured(connection, catalog) ??
    (billing?.credit_billing_configured === true ||
    billing?.charge_status === "usage_based"
      ? true
      : undefined);
  if (serviceConfigured === false) return "not_billable";
  if (serviceConfigured !== true) return "unknown";
  switch (credentialSupplier(connection, billing)) {
    case "nyxid":
      return "platform";
    case "own":
      return "byok";
    case "none": {
      const charge = connectionBillability(connection, billing, catalog);
      return charge === false
        ? "not_billable"
        : charge === true
          ? "platform"
          : "unknown";
    }
    default:
      return "unknown";
  }
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
