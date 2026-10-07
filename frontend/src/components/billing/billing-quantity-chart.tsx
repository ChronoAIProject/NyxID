import { useState } from "react";
import { ChevronDown } from "lucide-react";
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";
import { AnalyticsSelect } from "@/components/billing-analytics/filter-picker";
import { Button } from "@/components/ui/button";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { BILLING_UNITS } from "@/schemas/billing-metrics";
import type { BillingUsageRow } from "@/schemas/billing";
import { billingMetricLabel } from "@/lib/billing-units";
import { compact, number, type BillingCatalog } from "@/lib/billing-display";
import {
  dimensions,
  groupRows,
  metricTotals,
  type Dimension,
} from "@/lib/billing-usage";
import { BillingMetricPicker } from "./billing-metric-picker";

const tokenOrder = [
  "input_tokens",
  "output_tokens",
  "cache_read_tokens",
  "cache_write_tokens",
  "tokens",
];
const tickLabels: Record<string, string> = {
  tokens: "Total",
  input_tokens: "Input",
  output_tokens: "Output",
  cache_read_tokens: "Cache read",
  cache_write_tokens: "Cache write",
};
const colors = [
  "var(--billing-chart-accent)",
  "var(--color-info)",
  "var(--color-success)",
  "var(--color-warning)",
  "var(--color-muted-foreground)",
];
const POINTS_PER_PAGE = 8;

function unitFamily(metric: string) {
  return BILLING_UNITS[metric as keyof typeof BILLING_UNITS]?.tokenFamily
    ? "tokens"
    : metric;
}

function preferredComparison(
  catalog: BillingCatalog,
  rows: BillingUsageRow[],
  metric: string,
): Dimension {
  const metricRows = rows.filter((row) => row.metric === metric);
  for (const dimension of ["model", "agent", "layer"] as const) {
    if (groupRows(catalog, metricRows, dimension).length > 1) return dimension;
  }
  return "metric";
}

function QuantityPanel({
  rows,
  catalog,
  metrics,
  unit,
  comparison,
  onChooseMetrics,
}: {
  rows: BillingUsageRow[];
  catalog: BillingCatalog;
  metrics: string[];
  unit: string;
  comparison: Dimension;
  onChooseMetrics: () => void;
}) {
  const [page, setPage] = useState(0);
  const empty = metrics.length === 0;
  const totals = metricTotals(rows);
  const series = metrics.map((metric) => ({
    key: metric,
    label: billingMetricLabel(metric),
    color: colors[Math.max(0, tokenOrder.indexOf(metric)) % colors.length]!,
  }));
  const points =
    comparison === "metric"
      ? totals.map(([metric, quantity]) => ({
          key: metric,
          name: billingMetricLabel(metric),
          tick: tickLabels[metric] ?? billingMetricLabel(metric),
          quantity,
          values: { quantity } as Record<string, number | null>,
        }))
      : groupRows(catalog, rows, comparison)
          .sort((a, b) =>
            a.name.localeCompare(b.name, undefined, { numeric: true }),
          )
          .map((group) => {
            const quantities = new Map(metricTotals(group.rows));
            const values = Object.fromEntries(
              metrics.map((metric) => [metric, quantities.get(metric) ?? null]),
            );
            return {
              ...values,
              key: group.key,
              name: group.name,
              tick: group.name,
              values,
            };
          });
  const plottedSeries =
    comparison === "metric" && !empty
      ? [{ key: "quantity", label: "Period quantity", color: colors[0]! }]
      : series;
  const chartConfig = Object.fromEntries(
    plottedSeries.map((item) => [
      item.key,
      { label: item.label, color: item.color },
    ]),
  ) satisfies ChartConfig;
  const lastPage = Math.max(0, Math.ceil(points.length / POINTS_PER_PAGE) - 1);
  const currentPage = Math.min(page, lastPage);
  const visiblePoints = points.slice(
    currentPage * POINTS_PER_PAGE,
    (currentPage + 1) * POINTS_PER_PAGE,
  );
  const label = empty ? "Quantity" : billingMetricLabel(unit);
  const yMax = Math.max(
    1,
    ...points.flatMap((point) =>
      Object.values(point.values).map((value) => value ?? 0),
    ),
  );
  return (
    <section className="quantity-unit-panel" aria-label={`${label} comparison`}>
      {!empty && (
        <dl className="quantity-chart-totals">
          {series.map((item) => (
            <div key={item.key}>
              <dt className="capitalize">
                <span
                  className="quantity-series-swatch"
                  style={{
                    background:
                      comparison === "metric" ? colors[0] : item.color,
                  }}
                />
                {item.label}
              </dt>
              <dd>
                {number(
                  totals.find(([metric]) => metric === item.key)?.[1] ?? 0,
                )}
              </dd>
            </div>
          ))}
        </dl>
      )}
      <div className="quantity-axis-label capitalize">{label}</div>
      <div className="relative">
        <div
          role="img"
          aria-label={
            empty
              ? `Empty quantity graph by ${dimensions[comparison].toLowerCase()}: ${visiblePoints.map((point) => point.name).join("; ")}. No metrics selected.`
              : `${label} by ${dimensions[comparison].toLowerCase()} for the selected period: ${visiblePoints.map((point) => `${point.name}: ${plottedSeries.map((item) => `${item.label} ${point.values[item.key] == null ? "not recorded" : number(point.values[item.key]!)}`).join(", ")}`).join("; ")}`
          }
        >
          <ChartContainer
            config={chartConfig}
            className="h-[200px] w-full aspect-auto"
          >
            <LineChart
              data={visiblePoints}
              margin={{ top: 12, right: 14, left: 0, bottom: 0 }}
            >
              <CartesianGrid vertical={false} strokeDasharray="3 3" />
              <XAxis
                dataKey="key"
                tickLine={false}
                axisLine={false}
                tick={{ fontSize: 10, fill: "var(--color-muted-foreground)" }}
                minTickGap={16}
                tickFormatter={(value) => {
                  const text =
                    visiblePoints.find((point) => point.key === value)?.tick ??
                    value;
                  return text.length > 16 ? `${text.slice(0, 14)}…` : text;
                }}
              />
              <YAxis
                tickFormatter={compact}
                tickLine={false}
                axisLine={false}
                width={44}
                tick={
                  empty
                    ? false
                    : { fontSize: 10, fill: "var(--color-muted-foreground)" }
                }
                domain={[0, empty ? 1 : yMax]}
                ticks={empty ? [0, 0.25, 0.5, 0.75, 1] : undefined}
              />
              {!empty && (
                <ChartTooltip
                  content={
                    <ChartTooltipContent
                      labelFormatter={(_, payload) => payload[0]?.payload?.name}
                    />
                  }
                />
              )}
              {plottedSeries.map((item, index) => (
                <Line
                  key={item.key}
                  dataKey={item.key}
                  name={item.key}
                  type="linear"
                  stroke={item.color}
                  strokeWidth={1.5}
                  strokeDasharray={index > 1 ? "4 2" : undefined}
                  dot={{ r: 3, fill: item.color, strokeWidth: 0 }}
                  activeDot={{ r: 5 }}
                  isAnimationActive={false}
                />
              ))}
            </LineChart>
          </ChartContainer>
        </div>
        {empty && (
          <div className="pointer-events-none absolute inset-x-4 top-3 bottom-8 flex items-center justify-center">
            <div className="pointer-events-auto max-w-xs space-y-2 rounded-lg border border-border bg-card p-3 text-center shadow-sm">
              <p className="text-12 font-medium text-foreground">
                Select metrics to display usage
              </p>
              <p className="text-11 text-muted-foreground">
                Use the Metrics filter to select one or more metrics.
              </p>
              <Button variant="outline" size="sm" onClick={onChooseMetrics}>
                Choose metrics
              </Button>
            </div>
          </div>
        )}
      </div>
      <div className="quantity-category-axis">{dimensions[comparison]}</div>
      {lastPage > 0 && (
        <div className="quantity-chart-pagination">
          <span>
            {currentPage * POINTS_PER_PAGE + 1}–
            {Math.min((currentPage + 1) * POINTS_PER_PAGE, points.length)} of{" "}
            {points.length} categories
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={currentPage === 0}
            onClick={() => setPage(currentPage - 1)}
          >
            Previous categories
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={currentPage === lastPage}
            onClick={() => setPage(currentPage + 1)}
          >
            Next categories
          </Button>
        </div>
      )}
    </section>
  );
}

export function BillingQuantityChart({
  rows,
  catalog,
  overview = false,
}: {
  rows: BillingUsageRow[];
  catalog: BillingCatalog;
  overview?: boolean;
}) {
  const totals = metricTotals(rows);
  const metrics = totals.map(([metric]) => metric);
  const defaultMetric =
    ["input_tokens", "tokens", "requests", "output_tokens"].find((metric) =>
      metrics.includes(metric),
    ) ??
    metrics[0] ??
    "";
  const [comparison, setComparison] = useState<Dimension>(() =>
    overview ? "service" : preferredComparison(catalog, rows, defaultMetric),
  );
  const [selectedMetrics, setSelectedMetrics] = useState<string[]>(() =>
    defaultMetric ? [defaultMetric] : [],
  );
  const [pickerOpen, setPickerOpen] = useState(false);
  const selected = selectedMetrics.filter((metric) => metrics.includes(metric));
  const units = [...new Set(selected.map(unitFamily))];
  if (!totals.length) return null;
  return (
    <div className="usage-quantity-chart">
      <header>
        <div>
          <h4>Quantity overview</h4>
          <p className="fine-print">Totals for the selected billing period</p>
        </div>
      </header>
      <BillingMetricPicker
        open={pickerOpen}
        onOpenChange={setPickerOpen}
        values={selected}
        onChange={setSelectedMetrics}
        options={totals.map(([metric, quantity]) => ({
          id: metric,
          label: billingMetricLabel(metric),
          detail: `${number(quantity)} in this period`,
        }))}
      >
        <AnalyticsSelect
          label="Compare quantities by"
          inline
          value={comparison}
          onChange={(value) => setComparison(value as Dimension)}
          options={[
            ...(overview ? ["service"] : []),
            "metric",
            "model",
            "agent",
            "layer",
          ].map((value) => ({
            value,
            label: dimensions[value as Dimension],
          }))}
        />
      </BillingMetricPicker>
      {units.length === 0 ? (
        <QuantityPanel
          key={`empty:${comparison}`}
          rows={rows}
          catalog={catalog}
          metrics={[]}
          unit=""
          comparison={comparison}
          onChooseMetrics={() => setPickerOpen(true)}
        />
      ) : (
        units.map((unit) => {
          const unitMetrics = selected.filter(
            (metric) => unitFamily(metric) === unit,
          );
          return (
            <QuantityPanel
              key={`${comparison}:${unitMetrics.join(",")}`}
              rows={rows.filter((row) => unitMetrics.includes(row.metric))}
              catalog={catalog}
              metrics={unitMetrics}
              unit={unit}
              comparison={comparison}
              onChooseMetrics={() => setPickerOpen(true)}
            />
          );
        })
      )}
      {selected.length > 0 && (
        <p className="fine-print">
          Each point is a period total for one{" "}
          {dimensions[comparison].toLowerCase()}. Token metrics can overlap;
          each metric is shown separately. Different units use separate graphs.
        </p>
      )}
      <details className="quantity-exact-values">
        <summary>
          <ChevronDown
            size={12}
            className="disclosure-arrow"
            aria-hidden="true"
          />
          Exact quantities
        </summary>
        <dl>
          {totals.map(([metric, quantity]) => (
            <div key={metric}>
              <dt className="capitalize">{billingMetricLabel(metric)}</dt>
              <dd>{number(quantity)}</dd>
            </div>
          ))}
        </dl>
      </details>
    </div>
  );
}
