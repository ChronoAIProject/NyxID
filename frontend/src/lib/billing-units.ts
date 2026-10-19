import { metricLabel } from "@/schemas/billing-metrics";
import type { BillingMetric } from "@/schemas/billing";
import type { AllowanceForm } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";

type AllowanceRecurrence = AllowanceForm["recurrence"];

const RECURRENCE_PHRASES: Readonly<Record<AllowanceRecurrence, string>> = {
  one_time: "once",
  daily: "each day",
  weekly: "each week",
  monthly: "each month",
};

/**
 * The backend owns metric resolution. Keeping this access in one utility
 * prevents components from inspecting billing config, slugs, or protocols.
 */
export function resolveServiceBillingMetric(
  service: Pick<DownstreamService, "effective_platform_metric">,
): BillingMetric {
  return service.effective_platform_metric;
}

export function billingMetricLabel(metric: string, quantity?: number): string {
  return metricLabel(metric, quantity);
}

export function formatAllowancePreview(
  quantity: number,
  metric: string,
  recurrence: AllowanceRecurrence,
  locale?: string,
): string | null {
  if (!Number.isInteger(quantity) || quantity <= 0) return null;

  const formatted = new Intl.NumberFormat(locale).format(quantity);
  const compact = new Intl.NumberFormat(locale, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(quantity);
  const compactSuffix = compact === formatted ? "" : ` (${compact})`;

  return `${formatted} ${billingMetricLabel(metric, quantity)}${compactSuffix} free ${RECURRENCE_PHRASES[recurrence]}`;
}

/** Credential labels for platform-wide reporting; preserve future classes. */
const CREDENTIAL_CLASS_LABELS: Readonly<Record<string, string>> = {
  nyxid_managed_master: "NyxID platform key",
  user_owned: "User's own key (BYOK)",
  agent_override_user_owned: "Own key · agent override",
  node_managed: "Own key · node-managed",
  nyxid_platform_oauth_app: "Shared OAuth app",
  no_auth: "No authentication",
};

export function credentialClassLabel(credentialClass: string): string {
  return CREDENTIAL_CLASS_LABELS[credentialClass] ?? credentialClass;
}
