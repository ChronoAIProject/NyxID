import { metricLabel } from "@/schemas/billing-metrics";
import type {
  ServiceBillingExplanation,
  ServiceCaller,
  ServiceInsight,
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
        bill.charge_status !== first.charge_status,
    )
  )
    return "Varies by connection";
  return first.account
    ? `For you: ${billingAccountLabel(first)}`
    : "Depends on execution";
}
