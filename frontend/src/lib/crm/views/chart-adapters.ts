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
    typeof point.key === "boolean"
      ? point.key
        ? "Yes"
        : "No"
      : point.label;
  return {
    key:
      typeof point.key === "boolean"
        ? `boolean:${String(point.key)}`
        : (point.key ?? "__none__"),
    label:
      point.archived && point.key !== "__other__"
        ? `${label} (archived)`
        : label,
    value: point.value,
    archived: point.archived,
  };
}

export function aggregateResultToChartPoints(
  result: Pick<CrmAggregateResult, "points">,
): readonly CrmChartPoint[] {
  return result.points.map(aggregatePointToChartPoint);
}

export function formatAggregateValue(
  value: number | null,
  unit: CrmAggregateResult["unit"],
): string {
  if (value === null) return "No value";

  if (unit.format === "currency" && unit.currency) {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: unit.currency,
    }).format(value / 100);
  }

  return new Intl.NumberFormat(undefined, {
    maximumFractionDigits: unit.format === "count" ? 0 : 2,
  }).format(value);
}
