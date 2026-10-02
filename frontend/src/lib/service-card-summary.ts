import type {
  ServiceBillingExplanation,
  ConfiguredCatalogEntry,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";
import {
  configuredUsageCharge,
  positiveUsageRate,
} from "./service-billing-config";

export function connectionBillability(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
  catalog?: ConfiguredCatalogEntry,
): boolean | undefined {
  if (billing?.status === "restricted") return undefined;
  if (billing?.credit_billing_configured != null)
    return billing.credit_billing_configured;
  const configured = configuredUsageCharge(
    connection,
    catalog,
    billing?.credential_class,
  );
  if (configured !== undefined) return configured;
  if (billing?.status === "unavailable") return undefined;
  if (billing?.rates.length)
    return billing.rates.some(
      (rate) =>
        rate.credits_per_unit == null ||
        positiveUsageRate(rate.credits_per_unit),
    );
  if (billing?.charge_status === "usage_based") return true;
  // "Not charged" can describe caller rollout rather than the service's configuration.
  return undefined;
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
