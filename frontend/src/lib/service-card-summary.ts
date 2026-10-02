import type {
  ServiceBillingExplanation,
  ConfiguredCatalogEntry,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";
import {
  configuredUsageCharge,
  configuredPlatformPrice,
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
  not_billable: "Not billable",
  unknown: "Unverified",
};

export function connectionBillingCategory(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
  catalog?: ConfiguredCatalogEntry,
): ConnectionBillingCategory {
  if (billing?.status === "restricted") return "unknown";
  // A resolved caller override takes precedence over the connection default.
  switch (billing?.credential_class) {
    case "nyxid_managed_master":
    case "nyxid_platform_oauth_app":
      return "platform";
    case "user_owned":
    case "agent_override_user_owned":
    case "node_managed":
      return "byok";
    case "no_auth":
      return connectionBillability(connection, billing, catalog) === false
        ? "not_billable"
        : "unknown";
  }
  if (billing?.context === "agent_key") return "unknown";
  if (connection.credential_binding === "platform") return "platform";
  if (billing?.provider_billing === "nyxid_credential") return "platform";
  // A stored user-key row can coexist with platform billing. It does not
  // establish who supplied that credential on an older server.
  if (configuredPlatformPrice(connection, catalog)) return "unknown";
  if (connection.node_id || connection.has_node_binding) return "unknown";
  if (connection.auth_method === "none")
    return connectionBillability(connection, billing, catalog) === false
      ? "not_billable"
      : "unknown";
  // An OAuth login does not establish who supplies the developer app.
  if (["oauth2", "device_code"].includes(connection.credential_type))
    return "unknown";
  if (
    connection.credential_type === "api_key" &&
    !connection.credential_missing &&
    (connection.api_key_id || connection.credential_binding === "user")
  )
    return "byok";
  return "unknown";
}

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
