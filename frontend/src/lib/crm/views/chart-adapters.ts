import type {
  CrmAggregatePoint,
  CrmAggregateResult,
} from "@/lib/crm/views/schema";

export interface CrmChartPoint {
  readonly key: string;
  readonly label: string;
  readonly value: number;
}

export function aggregatePointToChartPoint(
  point: CrmAggregatePoint,
): CrmChartPoint {
  return {
    key: point.key ?? "__none__",
    label: point.label,
    value: point.value,
  };
}

export function aggregateResultToChartPoints(
  result: Pick<CrmAggregateResult, "points">,
): readonly CrmChartPoint[] {
  return result.points.map(aggregatePointToChartPoint);
}

export function formatAggregateValue(
  value: number,
  unit: CrmAggregateResult["unit"],
): string {
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
