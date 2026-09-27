import { useId, type CSSProperties } from "react";
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
import "./visualization.css";

const COLORS = [
  "var(--analytics-violet)",
  "var(--analytics-blue)",
  "var(--analytics-lavender)",
  "var(--analytics-indigo)",
  "var(--analytics-sky)",
  "var(--analytics-periwinkle)",
  "var(--analytics-steel)",
  "var(--analytics-iris)",
  "var(--analytics-mist)",
  "var(--analytics-slate)",
] as const;
const seriesColor = (index: number, other: boolean) =>
  other
    ? "var(--analytics-other)"
    : (COLORS[index % COLORS.length] ?? COLORS[0]);
function ChartGradients({
  id,
  colors,
  horizontal = false,
}: {
  id: string;
  colors: string[];
  horizontal?: boolean;
}) {
  return (
    <defs>
      {colors.map((color, index) => (
        <linearGradient
          key={`area-${index}`}
          id={`${id}-area-${index}`}
          x1="0"
          y1="0"
          x2="0"
          y2="1"
        >
          <stop
            offset="0%"
            stopColor={color}
            stopOpacity="var(--analytics-area-top)"
          />
          <stop
            offset="55%"
            stopColor={color}
            stopOpacity="var(--analytics-area-middle)"
          />
          <stop offset="100%" stopColor={color} stopOpacity={0} />
        </linearGradient>
      ))}
      {colors.map((color, index) => (
        <linearGradient
          key={`bar-${index}`}
          id={`${id}-bar-${index}`}
          x1="0"
          y1={horizontal ? "0" : "1"}
          x2={horizontal ? "1" : "0"}
          y2="0"
        >
          <stop
            offset="0%"
            stopColor={color}
            stopOpacity="var(--analytics-bar-start)"
          />
          <stop
            offset="100%"
            stopColor={color}
            stopOpacity="var(--analytics-bar-end)"
          />
        </linearGradient>
      ))}
    </defs>
  );
}
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
  const chartId = useId().replace(/:/g, "");
  const seriesData = data.points.map((point, index) => ({
    ...point,
    ...Object.fromEntries(
      data.series.map((series, seriesIndex) => [
        `series${seriesIndex}`,
        series.points[index]?.value ?? null,
      ]),
    ),
  }));
  const temporal = panel.chart === "line" || panel.chart === "combo";
  const color = (index: number, other: boolean) =>
    seriesColor(panel.chart === "bar" ? 0 : index, other);
  const format = (value: number | null, compact = false) =>
    formatAnalyticsValue(value, data.unit, compact);
  const unit = unitLabel(data.unit);
  const unknown = data.totals.unknown_cost_events;
  const pieData = data.slices.filter(
    (slice) => slice.value !== null && slice.value > 0,
  );
  const noUsage = data.totals.events === 0;
  const tick = {
    fill: "var(--analytics-label)",
    fontSize: 11,
    fontFamily: "var(--font-sans)",
  };
  const seriesColors = data.series.map((series, index) =>
    color(index, series.is_other),
  );
  const tooltipStyle: CSSProperties = {
    background: "var(--color-popover)",
    border: "1px solid var(--color-border)",
    borderRadius: 10,
    padding: "10px 12px",
    boxShadow: "0 8px 24px rgb(0 0 0 / 16%)",
    color: "var(--color-foreground)",
    fontSize: 12,
    maxWidth: "min(420px, calc(100vw - 32px))",
    whiteSpace: "normal",
    overflowWrap: "anywhere",
  };
  const tooltipTextStyle = { color: "var(--color-popover-foreground)" };
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
      <span className="text-[15px] font-medium text-foreground">
        No usage in this view
      </span>
      <span className="text-[12px]">
        Try a different time range or clear a filter.
      </span>
    </div>
  ) : panel.chart === "pie" ? (
    pieData.length ? (
      <ResponsiveContainer
        width="100%"
        height="100%"
        minWidth={1}
        initialDimension={{ width: 640, height: 280 }}
      >
        <PieChart accessibilityLayer>
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
                fill={color(data.slices.indexOf(slice), slice.is_other)}
              />
            ))}
          </Pie>
          <Tooltip
            contentStyle={tooltipStyle}
            itemStyle={tooltipTextStyle}
            labelStyle={tooltipTextStyle}
            filterNull={false}
            formatter={(v, name) => [
              `${format(v == null ? null : Number(v))} ${unit}`,
              name,
            ]}
          />
        </PieChart>
      </ResponsiveContainer>
    ) : (
      <div className="flex h-full items-center justify-center text-[12px] text-muted-foreground">
        {data.slices.some((slice) => slice.value === null)
          ? "Cost is unavailable for this selection."
          : "No positive values to display."}
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
        accessibilityLayer
        data={data.slices}
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
          width={105}
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
          formatter={(v) => [`${format(v == null ? null : Number(v))} ${unit}`]}
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
            minTickGap={35}
            interval="preserveStartEnd"
            tickFormatter={(s: string) =>
              `${bucketLabel(s, data.granularity)}${partialBucket(s, data) ? "*" : ""}`
            }
          />
          <YAxis
            yAxisId="value"
            tick={tick}
            width={55}
            tickLine={false}
            axisLine={false}
            tickFormatter={(n: number) => format(n, true)}
          />
          <YAxis
            yAxisId="requests"
            orientation="right"
            tick={tick}
            width={45}
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
                : [`${format(v == null ? null : Number(v))} ${unit}`, name]
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
            minTickGap={35}
            interval="preserveStartEnd"
            tickFormatter={(s: string) =>
              `${bucketLabel(s, data.granularity)}${partialBucket(s, data) ? "*" : ""}`
            }
          />
          <YAxis
            tick={tick}
            width={55}
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
            formatter={(v, name) => [
              `${format(v == null ? null : Number(v))} ${unit}`,
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
            className={cn(
              "font-display font-medium tracking-tight tabular-nums",
              compact ? "text-[22px]" : "text-[28px]",
            )}
          >
            {format(data.total, true)}
          </span>
          <span className="ml-2 text-[11px] text-muted-foreground">{unit}</span>
        </div>
        <span className="text-[10px] text-muted-foreground">
          {temporal
            ? `${INTERVAL_LABELS[data.granularity]} · UTC`
            : panel.top
              ? `Top ${panel.top}${data.slices.some((s) => s.is_other) ? " + Other" : ""}`
              : "All usage"}
        </span>
      </div>
      {panel.chart === "combo" && (
        <div className="mb-2 flex justify-between text-[10px] text-muted-foreground">
          <span>Bars · {unit} · left axis</span>
          <span>Line · requests · right axis</span>
        </div>
      )}
      <div
        role="img"
        aria-label={`${panel.title}: ${panel.chart} chart. Values available in the data table.`}
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
        className="mt-3 space-y-1 text-[11px] text-muted-foreground"
      >
        {temporal &&
          data.points.some((point) => partialBucket(point.bucket, data)) && (
            <p>* Partial intervals. Hover for the exact UTC dates included.</p>
          )}
        {unknown > 0 && (
          <p className="text-warning">
            {unknown.toLocaleString()} metering events have unknown cost.
            Affected cost values are unavailable
            {panel.chart === "pie" ? "; slices show known values only" : ""}.
          </p>
        )}
        {data.totals.legacy_cost_events > 0 && data.unit === "microcredits" && (
          <p>Includes historical estimates at cached rates.</p>
        )}
        {!data.freshness.validated && (
          <p className="text-warning">
            Usage is updating. Refresh to confirm this snapshot.
          </p>
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
                "flex items-center justify-between gap-3 text-[11px]",
                compact && "max-w-[46%]",
              )}
              title={`${slice.label}: ${format(slice.value)} ${unit}`}
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
                    className="h-auto min-w-0 justify-start p-0 text-[11px] text-foreground"
                    title={`Filter to ${slice.label}`}
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
                {format(slice.value)}
              </span>
            </div>
          ))}
        </div>
      )}
      <div className={cn("pt-4", compact && "mt-auto")}>
        <details className="border-t border-border/50 pt-3">
          <summary className="cursor-pointer text-[11px] text-muted-foreground hover:text-foreground">
            View data table
          </summary>
          <div className="mt-3 max-h-72 overflow-auto rounded-lg border border-border/50">
            <table className="w-full text-left text-[11px]">
              <caption className="sr-only">{panel.title} data</caption>
              <thead className="bg-muted/40">
                <tr>
                  <th className="px-3 py-2 font-medium">
                    {temporal ? "Time (UTC)" : "Group"}
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
                    <th className="px-3 py-2 text-right font-medium">
                      Requests
                    </th>
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
                          {format(point.value)}
                        </td>
                        {panel.top !== 0 &&
                          data.series.map((series, index) => (
                            <td
                              key={index}
                              className="px-3 py-2 text-right font-mono"
                            >
                              {format(series.points[pointIndex]?.value ?? null)}
                            </td>
                          ))}
                        {panel.chart === "combo" && (
                          <td className="px-3 py-2 text-right font-mono">
                            {point.requests.toLocaleString()}
                          </td>
                        )}
                      </tr>
                    ))
                  : data.slices.map((slice, index) => (
                      <tr key={index} className="border-t border-border/40">
                        <td className="px-3 py-2">{slice.label}</td>
                        <td className="px-3 py-2 text-right font-mono">
                          {format(slice.value)}
                        </td>
                      </tr>
                    ))}
              </tbody>
            </table>
          </div>
        </details>
      </div>
    </div>
  );
}
