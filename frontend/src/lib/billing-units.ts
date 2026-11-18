import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi } from "@/i18n/use-ui-translation";
import { metricLabel } from "@/schemas/billing-metrics";
import type { BillingMetric } from "@/schemas/billing";
import type { AllowanceForm } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";

type AllowanceRecurrence = AllowanceForm["recurrence"];

const RECURRENCE_PHRASES: Readonly<Record<AllowanceRecurrence, string>> = {
  one_time: "{{quantity}} {{unit}}{{compact}} free once",
  daily: "{{quantity}} {{unit}}{{compact}} free each day",
  weekly: "{{quantity}} {{unit}}{{compact}} free each week",
  monthly: "{{quantity}} {{unit}}{{compact}} free each month",
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

  const formatted = new Intl.NumberFormat(locale ?? getFormattingLocale()).format(quantity);
  const compact = new Intl.NumberFormat(locale ?? getFormattingLocale(), {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(quantity);
  const compactSuffix = compact === formatted ? "" : ` (${compact})`;

  return translateUi(RECURRENCE_PHRASES[recurrence], { quantity: formatted, unit: billingMetricLabel(metric, quantity), compact: compactSuffix });
}

/** Credential labels for platform-wide reporting; preserve future classes. */
const CREDENTIAL_CLASS_LABELS: Readonly<Record<string, string>> = {
  get nyxid_managed_master() { return translateUi("NyxID platform key"); },
  get user_owned() { return translateUi("User's own key (BYOK)"); },
  get agent_override_user_owned() { return translateUi("Own key · agent override"); },
  get node_managed() { return translateUi("Own key · node-managed"); },
  get nyxid_platform_oauth_app() { return translateUi("Shared OAuth app"); },
  get no_auth() { return translateUi("No authentication"); },
};

export function credentialClassLabel(credentialClass: string): string {
  return CREDENTIAL_CLASS_LABELS[credentialClass] ?? credentialClass;
}
