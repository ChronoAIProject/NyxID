import { translateUi } from "@/i18n/use-ui-translation";
import type { BillingUsageRow } from "@/schemas/billing";
import { billingMetricLabel } from "@/lib/billing-units";
import {
  knownTotal,
  number,
  serviceName,
  type BillingCatalog,
} from "./billing-display";

export type Dimension =
  | "service"
  | "operation"
  | "model"
  | "agent"
  | "layer"
  | "metric";
export const dimensions: Record<Dimension, string> = {
  get service() { return translateUi("Service"); },
  get operation() { return translateUi("Operation"); },
  get model() { return translateUi("Model"); },
  get agent() { return translateUi("Agent"); },
  get layer() { return translateUi("Billing layer"); },
  get metric() { return translateUi("Metric"); },
};
export function groupRows(
  catalog: BillingCatalog,
  rows: BillingUsageRow[],
  dimension: Dimension,
) {
  const groups = new Map<
    string,
    { key: string; name: string; rows: BillingUsageRow[] }
  >();
  for (const row of rows) {
    const detailSuffix = row.detail_retained === false ? ":older-than-30-days" : "";
    const key =
      dimension === "service"
        ? `${row.service_id ?? row.service_slug ?? "unknown"}${detailSuffix}`
        : dimension === "operation"
          ? `${row.service_id ?? row.service_slug ?? "unknown"}:${row.operation ?? ""}${detailSuffix}`
          : dimension === "agent"
            ? row.org_channel_guest
              ? `org-channel:${row.org_channel_guest.channel_id}`
              : `${row.api_key_id ?? "unrecorded"}${detailSuffix}`
            : dimension === "model"
              ? `${row.model ?? "unrecorded"}${detailSuffix}`
              : dimension === "layer"
                ? `${row.layer}${detailSuffix}`
                : `${row.metric}${detailSuffix}`;
    const olderLabel = row.detail_retained === false ? ` · ${translateUi("Older than 30 days")}` : "";
    const name =
      dimension === "service"
        ? `${serviceName(catalog, row.service_slug)}${olderLabel}`
        : dimension === "operation"
          ? row.detail_retained === false
            ? `${serviceName(catalog, row.service_slug)}${olderLabel}`
            : `${serviceName(catalog, row.service_slug)} · ${row.operation ?? "Base price"}`
          : dimension === "agent"
            ? row.org_channel_guest
              ? `Channel guests · ${row.org_channel_guest.channel_id.slice(0, 8)}${olderLabel}`
              : row.detail_retained === false
                ? `${translateUi("Unavailable")}${olderLabel}`
                : `${row.api_key_name ??
                (row.api_key_id
                  ? "Unnamed agent key"
                  : "No agent key recorded")}${olderLabel}`
            : dimension === "model"
              ? `${row.model ?? "No model recorded"}${olderLabel}`
              : dimension === "layer"
                ? `${layerName(row.layer)}${olderLabel}`
                : `${billingMetricLabel(row.metric)}${olderLabel}`;
    const group = groups.get(key) ?? { key, name, rows: [] };
    group.rows.push(row);
    groups.set(key, group);
  }
  // Unpriced rows add nothing, so a partly unpriced group ranks by its lower bound.
  return [...groups.values()].sort((a, b) => {
    const left = knownTotal(a.rows, "estimated_credits_micros").sum;
    const right = knownTotal(b.rows, "estimated_credits_micros").sum;
    return right > left ? 1 : right < left ? -1 : a.name.localeCompare(b.name);
  });
}
export function layerName(layer: string) {
  return layer === "platform"
    ? translateUi("Platform")
    : layer === "resale"
      ? translateUi("Resale")
      : layer;
}
export function metricTotals(rows: BillingUsageRow[]) {
  const metrics = new Map<string, number>();
  for (const row of rows)
    metrics.set(row.metric, (metrics.get(row.metric) ?? 0) + row.quantity);
  return [...metrics.entries()];
}
export function quantitySummary(rows: BillingUsageRow[]) {
  const metrics = metricTotals(rows);
  return metrics.length <= 2
    ? metrics
        .map(
          ([metric, quantity]) =>
            `${number(quantity)} ${billingMetricLabel(metric)}`,
        )
        .join(" · ")
    : translateUi("{{count}} metered units · expand for quantities", { count: metrics.length });
}

export function usageStatus(rows: BillingUsageRow[]) {
  const charged = rows.filter((row) => row.billable);
  return {
    label: !charged.length
      ? "Free"
      : charged.some((row) => row.detail_retained === false)
        ? "Unavailable"
        : charged.some((row) => !row.lago_acked)
          ? "Pending"
          : "Acknowledged",
    mixed: charged.length > 0 && charged.length < rows.length,
  };
}

export function metricFamily(metric: string) {
  if (
    [
      "tokens",
      "input_tokens",
      "output_tokens",
      "reasoning_tokens",
      "image_input_tokens",
      "image_output_tokens",
      "audio_input_tokens",
      "audio_output_tokens",
    ].includes(metric)
  )
    return "Tokens";
  if (["cache_read_tokens", "cache_write_tokens"].includes(metric))
    return "Cache";
  return "Requests & other units";
}
