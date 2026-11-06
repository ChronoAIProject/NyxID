import { formatCrmMoney } from "@/lib/crm/money";
import type {
  CrmAggregatePoint,
  CrmAggregateResult,
} from "@/lib/crm/views/schema";

export interface CrmChartPoint {
  readonly key: string;
  readonly label: string;
  readonly value: number | null;
  readonly archived: boolean;
}

export function aggregatePointToChartPoint(
  point: CrmAggregatePoint,
): CrmChartPoint {
  const label =
    typeof point.key === "boolean" ? (point.key ? "Yes" : "No") : point.label;
  return {
    key:
      point.key === null ? "null" : `${typeof point.key}:${String(point.key)}`,
    label: point.archived ? `${label} (archived)` : label,
    value: point.value,
    archived: point.archived,
  };
}

export function aggregateResultToChartPoints(
  result: Pick<CrmAggregateResult, "points" | "truncated">,
): readonly CrmChartPoint[] {
  return result.points.map((point, index) => {
    const chartPoint = aggregatePointToChartPoint(point);
    // Truncated results append the synthetic remainder after all real groups.
    return result.truncated && index === result.points.length - 1
      ? { ...chartPoint, key: "remainder" }
      : chartPoint;
  });
}

export function formatAggregateValue(
  value: number | null,
  unit: CrmAggregateResult["unit"],
): string {
  if (value === null) return "No value";

  if (unit.format === "currency" && unit.currency) {
    return formatCrmMoney(value, unit.currency);
  }

  return new Intl.NumberFormat(undefined, {
    style: unit.format === "percent" ? "percent" : "decimal",
    maximumFractionDigits: unit.format === "count" ? 0 : 2,
  }).format(value);
}
