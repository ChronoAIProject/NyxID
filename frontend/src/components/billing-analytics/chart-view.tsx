import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { analyticsAmount, plotAmount } from "@/lib/usage-analytics";
import { useId } from "react";
import { useTextSize } from "@/hooks/use-text-size";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  ComposedChart,
  Line,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  bucketLabel,
  formatAnalyticsValue,
  unitLabel,
  INTERVAL_LABELS,
  BREAKDOWN_LABELS,
  PANEL_HEIGHTS,
  partialBucket,
  bucketBounds,
} from "@/lib/usage-analytics";
import type {
  AnalyticsPanel,
  AnalyticsResult,
} from "@/schemas/usage-analytics";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { ChevronDown } from "lucide-react";
import { ChartGradients } from "./chart-gradients";
import {
  chartTick,
  chartTooltipStyle,
  chartTooltipTextStyle,
} from "./chart-style";
import "./visualization.css";

const COLORS = [
  "var(--analytics-series-1)",
  "var(--analytics-series-2)",
  "var(--analytics-series-3)",
  "var(--analytics-series-4)",
  "var(--analytics-series-5)",
  "var(--analytics-series-6)",
  "var(--analytics-series-7)",
  "var(--analytics-series-8)",
  "var(--analytics-series-9)",
  "var(--analytics-series-10)",
  "var(--analytics-series-11)",
  "var(--analytics-series-12)",
  "var(--analytics-series-13)",
  "var(--analytics-series-14)",
  "var(--analytics-series-15)",
  "var(--analytics-series-16)",
  "var(--analytics-series-17)",
  "var(--analytics-series-18)",
  "var(--analytics-series-19)",
  "var(--analytics-series-20)",
] as const;
const seriesColor = (index: number, other: boolean) =>
  other
    ? "var(--analytics-other)"
    : (COLORS[index % COLORS.length] ?? COLORS[0]);
export function ChartView({
  data,
  panel,
  onSelect,
  compact = false,
}: {
  data: AnalyticsResult;
  panel: AnalyticsPanel;
  onSelect?: (id: string) => void;
  compact?: boolean;
}) {
  useUiTranslation();
  const chartId = useId().replace(/:/g, "");
  const tokenSize = useTextSize(13);
  const seriesData = data.points.map((point, index) => ({
    ...point,
    value: plotAmount(point),
    ...Object.fromEntries(
      data.series.map((series, indexOfSeries) => [
        `exactseries${indexOfSeries}`,
        analyticsAmount(series.points[index] ?? { value: null }),
      ]),
    ),
    ...Object.fromEntries(
      data.series.map((series, seriesIndex) => [
        `series${seriesIndex}`,
        plotAmount(series.points[index] ?? { value: null }),
      ]),
    ),
  }));
  const temporal = panel.chart === "line" || panel.chart === "combo";
  const color = seriesColor;
  const format = (value: string | number | null, compact = false) =>
    formatAnalyticsValue(value, data.unit, compact);
  const unit = unitLabel(data.unit);
  const unknown = data.totals.unknown_cost_events;
  const plotSlices = data.slices.map((slice, index) => ({
    ...slice,
    amount: analyticsAmount(slice),
    value: plotAmount(slice),
    index,
  }));
  const pieData = plotSlices.filter(
    (slice) => slice.value !== null && slice.value > 0,
  );
  const noUsage = data.totals.events === 0;
  // Recharts measures labels in JS. A CSS var alone leaves its measurement
  // cache unchanged when Display changes the root font size.
  const tick = { ...chartTick, fontSize: tokenSize };
  const seriesColors = data.series.map((series, index) =>
    color(index, series.is_other),
  );
  const tooltipStyle = chartTooltipStyle;
  const tooltipTextStyle = chartTooltipTextStyle;
  const timeLabel = (bucket: string) => {
    const label = `${bucketLabel(bucket, data.granularity, true)} UTC`;
    if (!partialBucket(bucket, data)) return label;
    const bounds = bucketBounds(bucket, data.granularity);
    const from = new Date(
      Math.max(bounds.start, Date.parse(data.window.from)),
    ).toISOString();
    const to = new Date(
      Math.min(bounds.end, Date.parse(data.window.to)),
    ).toISOString();
    return `${bucketLabel(from, "hour", true)} – ${bucketLabel(to, "hour", true)} UTC (partial)`;
  };
  const chart = noUsage ? (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
      <span className="type-card-title text-foreground">{translateUi("No usage in this view")}</span>
      <span className="text-14">{translateUi("Try a different time range or clear a filter.")}</span>
    </div>
  ) : panel.chart === "pie" ? (
    pieData.length ? (
      <ResponsiveContainer
        width="100%"
        height="100%"
        minWidth={1}
        initialDimension={{ width: 640, height: 280 }}
      >
        <PieChart key={tokenSize} accessibilityLayer>
          <Pie
            data={pieData}
            dataKey="value"
            nameKey="label"
            innerRadius="78%"
            outerRadius="92%"
            paddingAngle={pieData.length > 1 ? 2 : 0}
            cornerRadius={3}
            stroke="none"
            isAnimationActive={false}
          >
            {pieData.map((slice, index) => (
              <Cell
                key={`${slice.id}-${index}`}
                fill={color(slice.index, slice.is_other)}
              />
            ))}
          </Pie>
          <Tooltip
            contentStyle={tooltipStyle}
            itemStyle={tooltipTextStyle}
            labelStyle={tooltipTextStyle}
            filterNull={false}
            formatter={(_value, name, item) => [
              `${format(item.payload?.amount ?? null)} ${unit}`,
              name,
            ]}
          />
        </PieChart>
      </ResponsiveContainer>
    ) : (
      <div className="flex h-full items-center justify-center text-14 text-muted-foreground">
        {data.slices.some((slice) => slice.value === null)
          ? translateUi("Cost is unavailable for this selection.")
          : translateUi("No positive values to display.")}
      </div>
    )
  ) : panel.chart === "bar" ? (
    <ResponsiveContainer
      width="100%"
      height="100%"
      minWidth={1}
      initialDimension={{ width: 640, height: 280 }}
    >
      <BarChart
        key={tokenSize}
        accessibilityLayer
        data={plotSlices}
        layout="vertical"
        margin={{ top: 8, right: 20, bottom: 4, left: 0 }}
      >
        <ChartGradients
          id={chartId}
          colors={data.slices.map((slice, index) =>
            color(index, slice.is_other),
          )}
          horizontal
        />
        <CartesianGrid
          horizontal={false}
          stroke="var(--analytics-grid)"
          strokeWidth={0.6}
          strokeOpacity={0.5}
          strokeDasharray="4 6"
        />
        <XAxis
          type="number"
          tick={tick}
          tickLine={false}
          axisLine={false}
          tickFormatter={(n: number) => format(n, true)}
          interval="preserveStartEnd"
        />
        <YAxis
          type="category"
          dataKey="label"
          width={tokenSize * 9.5}
          tick={tick}
          tickLine={false}
          axisLine={false}
          tickFormatter={(s: string) =>
            s.length > 15 ? `${s.slice(0, 14)}…` : s
          }
        />
        <Tooltip
          contentStyle={tooltipStyle}
          itemStyle={tooltipTextStyle}
          labelStyle={tooltipTextStyle}
          filterNull={false}
          cursor={{ fill: "var(--color-muted)", opacity: 0.4 }}
          formatter={(_value, _name, item) => [
            `${format(item.payload?.amount ?? null)} ${unit}`,
          ]}
        />
        <Bar
          dataKey="value"
          name={unit}
          radius={[0, 3, 3, 0]}
          maxBarSize={14}
          stroke="none"
          isAnimationActive={false}
        >
          {data.slices.map((slice, index) => (
            <Cell
              key={`${slice.id}-${index}`}
              fill={`url(#${chartId}-bar-${index})`}
            />
          ))}
        </Bar>
      </BarChart>
    </ResponsiveContainer>
  ) : (
    <ResponsiveContainer
      width="100%"
      height="100%"
      minWidth={1}
      initialDimension={{ width: 640, height: 280 }}
    >
      {panel.chart === "combo" ? (
        <ComposedChart
          key={tokenSize}
          accessibilityLayer
          data={seriesData}
          margin={{ top: 10, right: 8, bottom: 4, left: 0 }}
        >
          <ChartGradients id={chartId} colors={seriesColors} />
          <CartesianGrid
            vertical={false}
            stroke="var(--analytics-grid)"
            strokeWidth={0.6}
            strokeOpacity={0.5}
            strokeDasharray="4 6"
          />
          <XAxis
            dataKey="bucket"
            tick={tick}
            tickLine={false}
            axisLine={false}
            minTickGap={tokenSize * 2}
            height={tokenSize * 2.5}
            interval="preserveStartEnd"
            tickFormatter={(s: string) =>
              `${bucketLabel(s, data.granularity)}${partialBucket(s, data) ? "*" : ""}`
            }
          />
          <YAxis
            yAxisId="value"
            tick={tick}
            width={tokenSize * 5}
            tickLine={false}
            axisLine={false}
            tickFormatter={(n: number) => format(n, true)}
          />
          <YAxis
            yAxisId="requests"
            orientation="right"
            tick={tick}
            width={tokenSize * 4}
            tickLine={false}
            axisLine={false}
            tickFormatter={(n: number) =>
              formatAnalyticsValue(n, "requests", true)
            }
          />
          <Tooltip
            contentStyle={tooltipStyle}
            itemStyle={tooltipTextStyle}
            labelStyle={tooltipTextStyle}
            filterNull={false}
            labelFormatter={(label) => timeLabel(String(label))}
            formatter={(v, name, item) =>
              item.dataKey === "requests"
                ? [formatAnalyticsValue(Number(v), "requests"), "Requests"]
                : [
                    `${format(item.payload?.[`exact${String(item.dataKey)}`] ?? null)} ${unit}`,
                    name,
                  ]
            }
          />
          {data.series.map((series, index) => (
            <Bar
              key={index}
              yAxisId="value"
              dataKey={`series${index}`}
              name={series.label}
              stackId="measure"
              fill={color(index, series.is_other)}
              fillOpacity={0.72}
              stroke="none"
              maxBarSize={24}
              isAnimationActive={false}
            />
          ))}
          <Line
            yAxisId="requests"
            dataKey="requests"
            name="Requests"
            type="monotone"
            stroke="var(--analytics-lavender)"
            strokeWidth={1.75}
            strokeLinecap="round"
            dot={data.points.length === 1}
            activeDot={{
              r: 4,
              fill: "var(--analytics-lavender)",
              stroke: "var(--color-card)",
              strokeWidth: 2,
            }}
            isAnimationActive={false}
          />
        </ComposedChart>
      ) : (
        <AreaChart
          key={tokenSize}
          accessibilityLayer
          data={seriesData}
          margin={{ top: 10, right: 20, bottom: 4, left: 0 }}
        >
          <ChartGradients id={chartId} colors={seriesColors} />
          <CartesianGrid
            vertical={false}
            stroke="var(--analytics-grid)"
            strokeWidth={0.6}
            strokeOpacity={0.5}
            strokeDasharray="4 6"
          />
          <XAxis
            dataKey="bucket"
            tick={tick}
            tickLine={false}
            axisLine={false}
            minTickGap={tokenSize * 2}
            height={tokenSize * 2.5}
            interval="preserveStartEnd"
            tickFormatter={(s: string) =>
              `${bucketLabel(s, data.granularity)}${partialBucket(s, data) ? "*" : ""}`
            }
          />
          <YAxis
            tick={tick}
            width={tokenSize * 5}
            tickLine={false}
            axisLine={false}
            tickFormatter={(n: number) => format(n, true)}
          />
          <Tooltip
            contentStyle={tooltipStyle}
            itemStyle={tooltipTextStyle}
            labelStyle={tooltipTextStyle}
            filterNull={false}
            labelFormatter={(label) => timeLabel(String(label))}
            formatter={(_value, name, item) => [
              `${format(item.payload?.[`exact${String(item.dataKey)}`] ?? null)} ${unit}`,
              name,
            ]}
          />
          {data.series.map((series, index) => (
            <Area
              key={index}
              dataKey={`series${index}`}
              name={series.label}
              type="monotone"
              fill={
                index < 2 && !series.is_other
                  ? `url(#${chartId}-area-${index})`
                  : "none"
              }
              fillOpacity={1}
              baseValue={0}
              stroke={color(index, series.is_other)}
              strokeOpacity={1}
              strokeDasharray={series.is_other ? "4 3" : undefined}
              strokeWidth={series.is_other ? 1.25 : index === 0 ? 2 : 1.5}
              strokeLinecap="round"
              dot={
                data.points.length === 1
                  ? {
                      r: 2,
                      strokeWidth: 0,
                      fill: color(index, series.is_other),
                    }
                  : false
              }
              activeDot={{
                r: 4.5,
                fill: color(index, series.is_other),
                stroke: "var(--color-card)",
                strokeWidth: 2,
              }}
              connectNulls={false}
              isAnimationActive={false}
            />
          ))}
        </AreaChart>
      )}
    </ResponsiveContainer>
  );
  return (
    <div
      className={cn(
        "analytics-visuals analytics-chart min-w-0",
        compact && "flex flex-1 flex-col",
      )}
    >
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <div>
          <span
            className="font-display type-metric tracking-tight tabular-nums"
          >
            {format(
              data.exact_total === undefined ? data.total : data.exact_total,
              true,
            )}
          </span>
          <span className="ml-2 text-13 text-muted-foreground">{unit}</span>
        </div>
        <span className="text-12 text-muted-foreground">
          {temporal
            ? translateUi("{{value1}} · UTC", { value1: INTERVAL_LABELS[data.granularity] })
            : panel.top
              ? translateUi("Top {{value1}}{{value2}}", { value1: panel.top, value2: data.slices.some((s) => s.is_other) ? translateUi(" + Other") : "" })
              : translateUi("All usage")}
        </span>
      </div>
      {panel.chart === "combo" && (
        <div className="mb-2 flex justify-between text-12 text-muted-foreground">
          <span>{translateUi("Bars · ")}{unit}{translateUi(" · left axis")}</span>
          <span>{translateUi("Line · requests · right axis")}</span>
        </div>
      )}
      <div
        role="img"
        aria-label={translateUi("{{value1}}: {{value2}} chart. Values available in the data table.", { value1: panel.title, value2: panel.chart })}
        aria-describedby={`${chartId}-coverage`}
        className={cn(
          "analytics-plot min-w-0 shrink-0",
          compact ? "h-[210px]" : "h-[260px] sm:h-[280px]",
        )}
        style={
          panel.height ? { height: PANEL_HEIGHTS[panel.height] } : undefined
        }
      >
        {chart}
      </div>
      <div
        id={`${chartId}-coverage`}
        className="mt-3 space-y-1 text-13 text-muted-foreground"
      >
        {temporal &&
          data.points.some((point) => partialBucket(point.bucket, data)) && (
            <p>{translateUi("* Partial intervals. Hover for the exact UTC dates included.")}</p>
          )}
        {unknown > 0 && (
          <p className="text-warning">
            {unknown.toLocaleString(getFormattingLocale())}{translateUi(" metering events have unknown cost. Affected cost values are unavailable")}{panel.chart === "pie" ? translateUi("; slices show known values only") : ""}.
          </p>
        )}
        {data.totals.legacy_cost_events > 0 && data.unit === "microcredits" && (
          <p>{translateUi("Includes historical estimates at cached rates.")}</p>
        )}
        {!data.freshness.validated && (
          <p className="text-warning">{translateUi("Usage is updating. Refresh to confirm this snapshot.")}</p>
        )}
      </div>
      {!noUsage && (
        <div
          className={cn(
            "mt-3",
            compact ? "flex flex-wrap gap-x-3 gap-y-1.5" : "space-y-1.5",
          )}
        >
          {data.slices.map((slice, index) => (
            <div
              key={`${slice.id}-${index}`}
              className={cn(
                "flex items-center justify-between gap-3 text-13",
                compact && "max-w-[46%]",
              )}
              title={translateUi("{{label}}: {{value}} {{unit}}", { label: slice.label, value: format(analyticsAmount(slice)), unit: unit })}
            >
              <div className="flex min-w-0 items-center gap-2">
                <span
                  className="size-1.5 shrink-0 rounded-full"
                  style={{
                    background: color(index, slice.is_other),
                  }}
                />
                {onSelect && slice.id && panel.top !== 0 ? (
                  <Button
                    variant="link"
                    className="h-auto min-w-0 justify-start p-0 text-14 text-foreground"
                    title={translateUi("Filter to {{value1}}", { value1: slice.label })}
                    onClick={() => onSelect(slice.id!)}
                  >
                    <span className="truncate">{slice.label}</span>
                  </Button>
                ) : (
                  <span className="truncate" title={slice.label}>
                    {slice.label}
                  </span>
                )}
              </div>
              <span
                className={cn(
                  "shrink-0 font-mono tabular-nums",
                  compact && "sr-only",
                )}
              >
                {format(analyticsAmount(slice))}
              </span>
            </div>
          ))}
        </div>
      )}
      <div className={cn("pt-4", compact && "mt-auto")}>
        <section
          className="border-t border-border/50 pt-3"
          aria-label={translateUi("{{value1}} data table", { value1: panel.title })}
        >
          <details
            key={panel.table_display ?? "always"}
            open={panel.table_display === "accordion" ? undefined : true}
            className="group"
          >
            <summary
              className={cn(
                "flex cursor-pointer list-none items-center justify-between type-secondary-emphasis text-muted-foreground [&::-webkit-details-marker]:hidden",
                panel.table_display !== "accordion" && "hidden",
              )}
            >{translateUi("Data table")}<ChevronDown className="size-3.5 transition-transform group-open:rotate-180" />
            </summary>
            {panel.table_display !== "accordion" && (
              <h4 className="type-secondary-emphasis text-muted-foreground">{translateUi("Data table")}</h4>
            )}
            <div
              className={cn(
                "mt-2 overflow-auto rounded-lg border border-border/50",
                compact ? "max-h-44" : "max-h-72",
              )}
            >
              <table className="w-full text-left text-13">
                <caption className="sr-only">{panel.title}{translateUi(" data")}</caption>
                <thead className="sticky top-0 bg-muted/90">
                  <tr>
                    <th className="px-3 py-2 font-medium">
                      {temporal
                        ? translateUi("Time (UTC)")
                        : panel.top === 0
                          ? translateUi("Group")
                          : BREAKDOWN_LABELS[panel.breakdown]}
                    </th>
                    <th className="px-3 py-2 text-right font-medium">{unit}</th>
                    {temporal &&
                      panel.top !== 0 &&
                      data.series.map((series, index) => (
                        <th
                          key={index}
                          className="whitespace-nowrap px-3 py-2 text-right font-medium"
                        >
                          {series.label}
                        </th>
                      ))}
                    {panel.chart === "combo" && (
                      <th className="px-3 py-2 text-right font-medium">{translateUi("Requests")}</th>
                    )}
                  </tr>
                </thead>
                <tbody>
                  {temporal
                    ? data.points.map((point, pointIndex) => (
                        <tr
                          key={point.bucket}
                          className="border-t border-border/40"
                        >
                          <td className="whitespace-nowrap px-3 py-2">
                            {timeLabel(point.bucket)}
                          </td>
                          <td className="px-3 py-2 text-right font-mono">
                            {format(analyticsAmount(point))}
                          </td>
                          {panel.top !== 0 &&
                            data.series.map((series, index) => (
                              <td
                                key={index}
                                className="px-3 py-2 text-right font-mono"
                              >
                                {format(
                                  analyticsAmount(
                                    series.points[pointIndex] ?? {
                                      value: null,
                                    },
                                  ),
                                )}
                              </td>
                            ))}
                          {panel.chart === "combo" && (
                            <td className="px-3 py-2 text-right font-mono">
                              {point.requests.toLocaleString(getFormattingLocale())}
                            </td>
                          )}
                        </tr>
                      ))
                    : data.slices.map((slice, index) => (
                        <tr key={index} className="border-t border-border/40">
                          <td className="px-3 py-2">{slice.label}</td>
                          <td className="px-3 py-2 text-right font-mono">
                            {format(analyticsAmount(slice))}
                          </td>
                        </tr>
                      ))}
                </tbody>
              </table>
            </div>
          </details>
        </section>
      </div>
    </div>
  );
}
