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

export function billingMetricLabel(
  metric: BillingMetric,
  quantity?: number,
): string {
  if (quantity === 1) {
    return metric === "bytes" ? "byte" : metric.slice(0, -1);
  }
  return metric;
}

export function formatAllowancePreview(
  quantity: number,
  metric: BillingMetric,
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
