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
  if (state === "loading") return `Loading ${field.toLowerCase()}…`;
  if (state === "restricted") return `${field} restricted`;
  if (state === "error") return `${field} couldn't load`;
  return `${field} not reported`;
}

export function credentialLabel(
  connection: KeyInfo,
  billing?: ServiceBillingExplanation | null,
): string {
  if (billing) return billing.credential_label;
  if (connection.credential_binding === "platform") return "NyxID credential";
  if (connection.node_id || connection.has_node_binding)
    return "Node credential";
  if (connection.auth_method === "none") return "No credential";
  if (connection.credential_source?.type === "org")
    return "Organization credential";
  return "Your credential";
}

export function billingAccountLabel(
  billing?: ServiceBillingExplanation | null,
): string {
  if (!billing) return "Billing not reported";
  if (billing.status === "restricted") return "Billing restricted";
  if (billing.context === "configuration" && billing.payer_rule)
    return billing.payer_rule;
  if (billing.charge_status === "not_charged") return "Not charged by NyxID";
  if (billing.status === "unavailable") return "Billing unavailable";
  if (billing.account)
    return billing.account.kind === "personal"
      ? "Personal account"
      : `${billing.account.name} · organization`;
  return billing.status === "conditional"
    ? "Depends on execution"
    : "Billing unavailable";
}

export function rateLabel(billing: ServiceBillingExplanation): string {
  if (billing.charge_status === "not_charged") return "No NyxID charge";
  if (billing.charge_status === "restricted") return "Rates restricted";
  if (!billing.rates.length) return "Rate not reported";
  if (billing.context === "configuration") {
    const pending = billing.rates.some((rate) => rate.sync_status !== "synced");
    const rate = billing.rates[0]!;
    const label =
      billing.rates.length === 1
        ? `${rate.credits_per_unit} credits / ${metricLabel(rate.metric, 1)} · configured`
        : `${billing.rates.length} configured rates`;
    return `${label}${pending ? " · sync unconfirmed" : ""}`;
  }
  if (billing.rates.length > 1) return `${billing.rates.length} metered rates`;
  const rate = billing.rates[0]!;
  return rate.credits_per_unit == null
    ? "Rate set at execution"
    : `${rate.credits_per_unit} credits / ${metricLabel(rate.metric, 1)}`;
}

export function callerKindLabel(kind: string): string {
  return (
    (
      {
        api_key: "Agent key",
        agent_key: "Agent key",
        app: "Application",
        oauth_app: "OAuth app",
        session: "Session",
        access_token: "Access token",
        service_account: "Service account",
        delegated: "Delegated token",
        relay: "Relay",
        unknown: "Caller not recorded",
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
        completed: "Completed",
        failed: "Failed",
        denied: "Denied",
        disconnected: "Disconnected",
        response_received: "Response received",
        connection_opened: "Connection opened",
        outcome_unknown: "Outcome unknown",
        unknown: "Outcome unknown",
      } as Record<string, string>
    )[outcome] ?? "Outcome unknown"
  );
}
export function accessReasonLabel(reason: string): string {
  return (
    (
      {
        selected: "Selected service",
        explicit: "Selected service",
        selected_service: "Selected service",
        all_services: "All services",
        platform_services: "Platform services",
        auto_connected: "Platform services",
      } as Record<string, string>
    )[reason] ?? reason.replaceAll("_", " ")
  );
}

export function accessCountLabel(access: ConnectionActivity["access"]): string {
  if (access.visibility === "unavailable") return "Key access unavailable";
  if (!access.keys.length && (access.incomplete || access.truncated))
    return "Key access incomplete";
  return `${access.keys.length}${access.incomplete || access.truncated ? "+" : ""} agent ${access.keys.length === 1 ? "key" : "keys"}`;
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
  if (request.source?.kind === "platform") return "Platform";
  if (request.source?.kind === "personal") return "Personal";
  if (request.source?.kind === "org") {
    const source = connection.credential_source;
    return source?.type === "org" && source.org_id === request.source.owner_id
      ? `Organization · ${source.org_name}`
      : "Organization";
  }
  return "Layer not recorded";
}

export function providerBillingLabel(
  billing?: ServiceBillingExplanation | null,
): string {
  if (billing?.provider_billing === "separate_provider_account")
    return "Provider billed separately";
  if (billing?.provider_billing === "nyxid_credential")
    return "NyxID supplies the credential";
  if (billing?.provider_billing === "no_credential")
    return "No provider credential";
  return "Provider billing not reported";
}

export function summarizeBillingDetail(
  insights: readonly (ServiceInsight | undefined)[],
): string {
  const bills = insights.map((item) => item?.billing);
  if (!bills.length || bills.some((bill) => !bill)) return "Rates unavailable";
  const first = bills[0]!;
  const rates = bills.every(
    (bill) =>
      JSON.stringify(bill!.rates) === JSON.stringify(first.rates) &&
      bill!.charge_status === first.charge_status,
  )
    ? rateLabel(first)
    : "Rates vary by connection";
  const providers = bills.every(
    (bill) => bill!.provider_billing === first.provider_billing,
  )
    ? providerBillingLabel(first)
    : "Provider billing varies";
  return `${rates} · ${providers}`;
}

export function summarizeBilling(
  insights: readonly (ServiceInsight | undefined)[],
): string {
  if (!insights.length || insights.some((item) => !item?.billing))
    return "Billing not reported";
  const bills = insights.map((item) => item!.billing!);
  if (bills.some((bill) => bill.status === "restricted"))
    return "Billing restricted";
  if (bills.every((bill) => bill.charge_status === "not_charged"))
    return "Not charged by NyxID";
  const first = bills[0]!;
  if (bills.every((bill) => bill.status === "unavailable"))
    return "Billing unavailable";
  if (bills.some((bill) => bill.status === "unavailable"))
    return "Varies by connection";
  if (
    bills.some(
      (bill) =>
        bill.account?.id !== first.account?.id ||
        bill.payer_rule !== first.payer_rule ||
        bill.charge_status !== first.charge_status,
    )
  )
    return "Varies by connection";
  if (first.context === "configuration")
    return `Expected: ${billingAccountLabel(first)}`;
  return first.account
    ? `For you: ${billingAccountLabel(first)}`
    : "Depends on execution";
}
