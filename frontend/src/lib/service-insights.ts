import { translateUi } from "@/i18n/use-ui-translation";
import { metricLabel } from "@/schemas/billing-metrics";
import type {
  ServiceBillingExplanation,
  ServiceCaller,
  ServiceInsight,
  ConnectionActivity,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";

export function insightStatusLabel(
  state: ServiceInsightsState["status"],
  field: "Access" | "Activity" | "Billing",
): string {
  if (state === "loading") return translateUi("Loading {{field}}…", { field: translateUi(field.toLowerCase()) });
  if (state === "restricted") return translateUi("{{field}} restricted", { field: translateUi(field) });
  if (state === "error") return translateUi("{{field}} couldn't load", { field: translateUi(field) });
  return translateUi("{{field}} not reported", { field: translateUi(field) });
}

export function credentialLabel(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
): string {
  if (billing) {
    if (billing.status === "restricted" || billing.status === "unavailable")
      return billing.credential_label;
    if (
      billing.context === "configuration" ||
      billing.context === "agent_key" ||
      billing.credential_supplier === "unknown"
    )
      return billing.credential_label;
    if (billing.credential_class === "nyxid_platform_oauth_app")
      return translateUi("NyxID developer app");
    if (billing.credential_class === "nyxid_managed_master") return translateUi("NyxID key");
    if (
      ["user_owned", "agent_override_user_owned"].includes(
        billing.credential_class ?? "",
      ) &&
      connection.credential_type === "api_key"
    )
      return translateUi(connection.credential_source?.type === "org" ? "Organization API key (BYOK)" : "Your API key (BYOK)");
    return billing.credential_label;
  }
  if (connection.credential_binding === "platform") return translateUi("NyxID credential");
  if (connection.node_id || connection.has_node_binding)
    return translateUi("Node credential");
  if (connection.auth_method === "none") return translateUi("No credential");
  if (connection.credential_source?.type === "org")
    return translateUi("Organization credential");
  return translateUi("Your credential");
}

export function billingAccountLabel(
  billing?: ServiceBillingExplanation | null,
): string {
  if (!billing) return translateUi("Billing not reported");
  if (billing.status === "restricted") return translateUi("Billing restricted");
  if (billing.context === "configuration" && billing.payer_rule)
    return billing.payer_rule;
  if (billing.charge_status === "not_charged") return translateUi("Not charged by NyxID");
  if (billing.status === "unavailable") return translateUi("Billing unavailable");
  if (billing.account)
    return billing.account.kind === "personal"
      ? translateUi("Personal account")
      : translateUi("{{name}} · organization", { name: billing.account.name });
  return billing.status === "conditional"
    ? translateUi("Depends on execution")
    : translateUi("Billing unavailable");
}

export function rateLabel(billing: ServiceBillingExplanation): string {
  if (billing.charge_status === "not_charged") return translateUi("No NyxID charge");
  if (billing.charge_status === "restricted") return translateUi("Rates restricted");
  if (!billing.rates.length) return translateUi("Rate not reported");
  if (billing.context === "configuration") {
    const pending = billing.rates.some((rate) => rate.sync_status !== "synced");
    const rate = billing.rates[0]!;
    const label =
      billing.rates.length === 1
        ? rate.credits_per_unit == null
          ? translateUi("Plan rate not reported")
          : translateUi("{{rate}} credits / {{metric}} · configured", { rate: rate.credits_per_unit, metric: metricLabel(rate.metric, 1) })
        : translateUi("{{count}} configured rates", { count: billing.rates.length });
    return `${label}${pending ? translateUi(" · sync unconfirmed") : ""}`;
  }
  if (billing.rates.length > 1) return translateUi("{{count}} metered rates", { count: billing.rates.length });
  const rate = billing.rates[0]!;
  return rate.credits_per_unit == null
    ? translateUi("Rate set at execution")
    : translateUi("{{rate}} credits / {{metric}}", { rate: rate.credits_per_unit, metric: metricLabel(rate.metric, 1) });
}

export function callerKindLabel(kind: string): string {
  return (
    (
      {
        api_key: translateUi("Agent key"),
        agent_key: translateUi("Agent key"),
        app: translateUi("Application"),
        oauth_app: translateUi("OAuth app"),
        session: translateUi("Session"),
        access_token: translateUi("Access token"),
        service_account: translateUi("Service account"),
        delegated: translateUi("Delegated token"),
        relay: translateUi("Relay"),
        unknown: translateUi("Caller not recorded"),
      } as Record<string, string>
    )[kind] ?? kind.replaceAll("_", " ")
  );
}
export function callerLabel(caller: ServiceCaller): string {
  return caller.name || callerKindLabel(caller.kind);
}
export function outcomeLabel(outcome: string): string {
  return (
    (
      {
        completed: translateUi("Completed"),
        failed: translateUi("Failed"),
        denied: translateUi("Denied"),
        disconnected: translateUi("Disconnected"),
        response_received: translateUi("Response received"),
        connection_opened: translateUi("Connection opened"),
        outcome_unknown: translateUi("Outcome unknown"),
        unknown: translateUi("Outcome unknown"),
      } as Record<string, string>
    )[outcome] ?? translateUi("Outcome unknown")
  );
}
export function accessReasonLabel(reason: string): string {
  return (
    (
      {
        selected: translateUi("Selected service"),
        explicit: translateUi("Selected service"),
        selected_service: translateUi("Selected service"),
        all_services: translateUi("All services"),
        platform_services: translateUi("Platform services"),
        auto_connected: translateUi("Platform services"),
      } as Record<string, string>
    )[reason] ?? reason.replaceAll("_", " ")
  );
}

export function accessCountLabel(access: ConnectionActivity["access"]): string {
  if (access.visibility === "unavailable") return translateUi("Key access unavailable");
  if (!access.keys.length && (access.incomplete || access.truncated))
    return translateUi("Key access incomplete");
  return translateUi(access.keys.length === 1 ? "{{count}}{{more}} agent key" : "{{count}}{{more}} agent keys", { count: access.keys.length, more: access.incomplete || access.truncated ? "+" : "" });
}

export function latestRecordedUse(usage?: ConnectionActivity | null) {
  if (usage?.activity.last_used !== undefined)
    return usage.activity.last_used ?? undefined;
  return usage?.activity.requests
    .filter(
      (request) =>
        request.outcome === "completed" ||
        (request.response_status != null &&
          ["response_received", "connection_opened", "failed"].includes(
            request.outcome,
          )),
    )
    .sort((a, b) => b.occurred_at.localeCompare(a.occurred_at))[0];
}

export function recordedSourceLabel(
  request: NonNullable<ReturnType<typeof latestRecordedUse>>,
  connection: KeyInfo,
): string {
  if (request.source?.kind === "platform") return translateUi("Platform");
  if (request.source?.kind === "personal") return translateUi("Personal");
  if (request.source?.kind === "org") {
    const source = connection.credential_source;
    return source?.type === "org" && source.org_id === request.source.owner_id
      ? translateUi("Organization · {{name}}", { name: source.org_name })
      : translateUi("Organization");
  }
  return translateUi("Layer not recorded");
}

export function providerBillingLabel(
  billing?: ServiceBillingExplanation | null,
): string {
  if (billing?.status === "restricted") return translateUi("Provider billing restricted");
  if (billing?.status === "unavailable") return translateUi("Provider billing unavailable");
  if (billing?.credential_supplier === "unknown")
    return translateUi("Credential supplier unverified");
  if (billing?.credential_class === "nyxid_platform_oauth_app")
    return translateUi("NyxID supplies the developer app; signing in connects your provider account");
  if (billing?.credential_supplier === "nyxid")
    return translateUi("NyxID supplies the key or developer app");
  if (billing?.provider_billing === "separate_provider_account")
    return translateUi("Provider billed separately");
  if (billing?.provider_billing === "nyxid_credential")
    return translateUi("NyxID supplies the credential");
  if (billing?.provider_billing === "no_credential")
    return translateUi("No provider credential");
  return translateUi("Provider billing not reported");
}

function billingModelCode(
  billing?: ServiceBillingExplanation | null,
): string {
  if (!billing) return "Billing not reported";
  if (billing.status === "restricted") return "Billing restricted";
  if (billing.status === "unavailable") return "Billing unavailable";
  if (billing.service_billing_configured === false)
    return "Not billable by NyxID";
  if (billing.credit_billing_configured === false)
    return "No NyxID charge for this connection";
  switch (billing.charge_status) {
    case "usage_based":
      return "NyxID credits";
    case "not_charged":
      return "No NyxID charge";
    case "restricted":
      return "Billing restricted";
    case "unavailable":
      return "Billing unavailable";
    default:
      return billing.context === "configuration" &&
        (billing.credit_billing_configured || billing.rates.length > 0)
        ? "NyxID credits · configured"
        : "Credit billing unverified";
  }
}


export function billingModelLabel(billing?: ServiceBillingExplanation | null): string {
  return translateUi(billingModelCode(billing));
}

export function summarizeBillingModel(
  status: ServiceInsightsState["status"],
  insights: readonly (ServiceInsight | undefined)[],
): string {
  if (status !== "ready") return insightStatusLabel(status, "Billing");
  const models = new Set(
    insights.map((item) => billingModelCode(item?.billing)),
  );
  if (!models.size) return translateUi("Billing not reported");
  if (models.size === 1) return translateUi([...models][0]!);
  if (
    [...models].some(
      (model) =>
        model.startsWith("Billing") || model === "Credit billing unverified",
    )
  )
    return translateUi("Billing partly reported");
  return translateUi("Charges vary by connection");
}

export function nyxidChargeLabel(billing: ServiceBillingExplanation): string {
  if (billing.status === "restricted") return translateUi("NyxID fees restricted");
  if (billing.status === "unavailable") return translateUi("NyxID fees unavailable");
  if (billing.charge_status === "not_charged") return translateUi("No NyxID charge");
  if (!billing.rates.length) return translateUi("Rate not reported");
  return translateUi("Rate: {{rate}}", { rate: rateLabel(billing) });
}

export function billingExplanation(billing: ServiceBillingExplanation): string {
  if (billing.status === "restricted" || billing.status === "unavailable")
    return billingModelLabel(billing);
  const supply =
    billing.credential_supplier === "unknown"
      ? translateUi("The supplier of this connection's key or developer app is unverified.")
      : billing.credential_supplier === "nyxid" &&
          billing.credential_class === "agent_override_user_owned"
        ? translateUi("The selected agent credential uses NyxID's developer app.")
        : billing.credential_class === "nyxid_platform_oauth_app"
          ? translateUi("NyxID supplies the developer app. Signing into your provider account is not BYOK.")
          : billing.provider_billing === "nyxid_credential"
            ? translateUi("NyxID supplies the provider key.")
            : billing.provider_billing === "separate_provider_account"
              ? translateUi("Your supplied credential uses a separate provider account. Any NyxID fees are additional to the provider's charges.")
              : billing.provider_billing === "no_credential"
                ? translateUi("No provider credential is required.")
                : translateUi("The supplier of this connection's key or developer app is unverified.");
  const charges =
    billing.credit_billing_configured === false
      ? translateUi("No NyxID usage charges are configured for this connection.")
      : billing.charge_status === "not_charged"
        ? translateUi("NyxID does not charge this caller for this connection.")
        : billing.charge_status === "usage_based"
          ? translateUi("NyxID meters usage against the billing account shown.")
          : billing.credit_billing_configured || billing.rates.length
            ? translateUi("NyxID credit billing is configured; caller eligibility and the active rate are verified at execution.")
            : translateUi("NyxID credit charges have not been verified. A missing rate does not mean usage is free.");
  return `${supply} ${charges}`;
}

export function summarizeBillingDetail(
  insights: readonly (ServiceInsight | undefined)[],
): string {
  const bills = insights.map((item) => item?.billing);
  if (!bills.length || bills.some((bill) => !bill)) return translateUi("Rates unavailable");
  if (bills.some((bill) => bill?.status === "restricted"))
    return translateUi("Billing details restricted");
  if (bills.some((bill) => bill?.status === "unavailable"))
    return translateUi("Billing details unavailable");
  const first = bills[0]!;
  const rates = bills.every(
    (bill) =>
      JSON.stringify(bill!.rates) === JSON.stringify(first.rates) &&
      bill!.charge_status === first.charge_status,
  )
    ? nyxidChargeLabel(first)
    : translateUi("NyxID fees vary by connection");
  const providers = bills.every(
    (bill) => bill!.provider_billing === first.provider_billing,
  )
    ? providerBillingLabel(first)
    : translateUi("Provider billing varies");
  return `${rates} · ${providers}`;
}

export function summarizeBilling(
  insights: readonly (ServiceInsight | undefined)[],
): string {
  if (!insights.length || insights.some((item) => !item?.billing))
    return translateUi("Billing not reported");
  const bills = insights.map((item) => item!.billing!);
  if (bills.some((bill) => bill.status === "restricted"))
    return translateUi("Billing restricted");
  if (bills.every((bill) => bill.charge_status === "not_charged"))
    return translateUi("Not charged by NyxID");
  const first = bills[0]!;
  if (bills.every((bill) => bill.status === "unavailable"))
    return translateUi("Billing unavailable");
  if (bills.some((bill) => bill.status === "unavailable"))
    return translateUi("Varies by connection");
  if (
    bills.some(
      (bill) =>
        bill.account?.id !== first.account?.id ||
        bill.payer_rule !== first.payer_rule ||
        bill.charge_status !== first.charge_status,
    )
  )
    return translateUi("Varies by connection");
  if (first.context === "configuration")
    return translateUi("Expected: {{account}}", { account: billingAccountLabel(first) });
  return first.account
    ? translateUi("For you: {{account}}", { account: billingAccountLabel(first) })
    : translateUi("Depends on execution");
}
