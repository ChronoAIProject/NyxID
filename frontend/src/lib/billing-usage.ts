import type { BillingUsageRow } from "@/schemas/billing";
import { billingMetricLabel } from "@/lib/billing-units";
import {
  number,
  serviceName,
  total,
  type BillingCatalog,
} from "./billing-display";

export type Dimension = "service" | "model" | "agent" | "layer" | "metric";
export const dimensions: Record<Dimension, string> = {
  service: "Service",
  model: "Model",
  agent: "Agent",
  layer: "Billing layer",
  metric: "Metric",
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
    const key =
      dimension === "service"
        ? (row.service_id ?? row.service_slug ?? "unknown")
        : dimension === "agent"
          ? (row.api_key_id ?? "unrecorded")
          : dimension === "model"
            ? (row.model ?? "unrecorded")
            : dimension === "layer"
              ? row.layer
              : row.metric;
    const name =
      dimension === "service"
        ? serviceName(catalog, row.service_slug)
        : dimension === "agent"
          ? (row.api_key_name ??
            (row.api_key_id ? "Unnamed agent key" : "No agent key recorded"))
          : dimension === "model"
            ? (row.model ?? "No model recorded")
            : dimension === "layer"
              ? layerName(row.layer)
              : billingMetricLabel(row.metric);
    const group = groups.get(key) ?? { key, name, rows: [] };
    group.rows.push(row);
    groups.set(key, group);
  }
  return [...groups.values()].sort(
    (a, b) =>
      (total(b.rows, "estimated_credits_micros") ?? -1) -
        (total(a.rows, "estimated_credits_micros") ?? -1) ||
      a.name.localeCompare(b.name),
  );
}
export function layerName(layer: string) {
  return layer === "platform"
    ? "Platform"
    : layer === "resale"
      ? "Resale"
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
    : `${metrics.length} metered units · expand for quantities`;
}

export function usageStatus(rows: BillingUsageRow[]) {
  const charged = rows.filter((row) => row.billable);
  return {
    label: !charged.length
      ? "Free"
      : charged.some((row) => !row.lago_acked)
        ? "Pending"
        : "Acknowledged",
    mixed: charged.length > 0 && charged.length < rows.length,
  };
}

export function metricFamily(metric: string) {
  if (["tokens", "input_tokens", "output_tokens"].includes(metric))
    return "Tokens";
  if (["cache_read_tokens", "cache_write_tokens"].includes(metric))
    return "Cache";
  return "Requests & other units";
}
