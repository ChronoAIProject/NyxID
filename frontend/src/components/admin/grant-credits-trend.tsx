import { useId } from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { FreeGrantsReport } from "@/schemas/free-grants-report";
import {
  chartTick,
  chartTooltipStyle,
} from "@/components/billing-analytics/chart-style";
import { ChartGradients } from "@/components/billing-analytics/chart-gradients";
import { useTextSize } from "@/hooks/use-text-size";
import "@/components/billing-analytics/visualization.css";

export function GrantCreditsTrend({ data }: { data: FreeGrantsReport }) {
  const id = useId().replace(/:/g, "");
  const tokenSize = useTextSize(13);
  const tick = { ...chartTick, fontSize: tokenSize };
  // Floating point is used only for chart coordinates. Labels, tooltips and
  // accessible data retain the server's exact amounts; no totals are derived here.
  const points = data.trend.map((bucket) => ({
    ...bucket,
    grantedPlot: Number(bucket.issued),
    usedPlot: Number(bucket.used),
  }));
  const colors = ["var(--analytics-series-1)", "var(--analytics-series-2)"];
  return (
    <div className="analytics-visuals analytics-chart min-w-0">
      <div className="mb-2 flex flex-wrap items-center gap-x-5 gap-y-1 type-secondary text-muted-foreground">
        <span>Daily · UTC · Credits</span>
        {colors.map((color, i) => (
          <span key={color} className="inline-flex items-center gap-1.5">
            <span
              aria-hidden="true"
              className="h-0.5 w-3"
              style={{ background: color }}
            />
            {i === 0 ? "Granted" : "Used"}
          </span>
        ))}
      </div>
      <div
        className="h-44"
        role="img"
        aria-label="Daily grant credits granted and used. Exact values are available in View chart data."
      >
        <ResponsiveContainer
          width="100%"
          height="100%"
          minWidth={1}
          initialDimension={{ width: 640, height: 176 }}
        >
          <AreaChart
            data={points}
            accessibilityLayer
            margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
          >
            <ChartGradients id={id} colors={colors} />
            <CartesianGrid
              vertical={false}
              stroke="var(--analytics-grid)"
              strokeDasharray="4 6"
            />
            <XAxis
              dataKey="start"
              tickFormatter={day}
              tick={tick}
              minTickGap={35}
              axisLine={false}
              tickLine={false}
            />
            <YAxis
              width={Math.ceil(tokenSize * 5)}
              tick={tick}
              tickFormatter={(v: number) =>
                v.toLocaleString(undefined, {
                  notation: "compact",
                  maximumSignificantDigits: 3,
                })
              }
              axisLine={false}
              tickLine={false}
            />
            <Tooltip
              content={({ active, payload }) => {
                const point = payload?.[0]?.payload as
                  | (typeof points)[number]
                  | undefined;
                return active && point ? (
                  <div style={chartTooltipStyle}>
                    <p className="mb-1 type-secondary-emphasis">
                      {utc(point.start)} – {utc(point.end)}
                    </p>
                    <p className="type-secondary">
                      Granted: <span className="font-mono">{point.issued}</span>{" "}
                      credits
                    </p>
                    <p className="type-secondary">
                      Used: <span className="font-mono">{point.used}</span>{" "}
                      credits
                    </p>
                  </div>
                ) : null;
              }}
            />
            <Area
              name="Granted"
              dataKey="grantedPlot"
              type="linear"
              stroke={colors[0]}
              strokeWidth={2}
              fill={`url(#${id}-area-0)`}
              isAnimationActive={false}
            />
            <Area
              name="Used"
              dataKey="usedPlot"
              type="linear"
              stroke={colors[1]}
              strokeWidth={2}
              fill={`url(#${id}-area-1)`}
              isAnimationActive={false}
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
      <details className="mt-2 type-secondary text-muted-foreground">
        <summary className="w-fit cursor-pointer">View chart data</summary>
        <div className="mt-2 overflow-x-auto">
          <table className="w-full text-left type-secondary">
            <caption className="sr-only">
              Exact daily grant credits. UTC bucket bounds are clipped to the
              selected window; end is exclusive.
            </caption>
            <thead>
              <tr>
                <th className="py-2 pr-3 type-table-header">UTC window</th>
                <th className="px-2 type-table-header">Granted</th>
                <th className="pl-2 type-table-header">Used</th>
              </tr>
            </thead>
            <tbody>
              {data.trend.map((b) => (
                <tr key={b.start} className="border-t border-border/50">
                  <th scope="row" className="py-2 pr-3 type-secondary">
                    {utc(b.start)} – {utc(b.end)}
                  </th>
                  <td className="break-all px-2 font-mono text-foreground">
                    {b.issued}
                  </td>
                  <td className="break-all pl-2 font-mono text-foreground">
                    {b.used}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </div>
  );
}
function day(value: string) {
  return new Date(value).toLocaleDateString(undefined, {
    timeZone: "UTC",
    month: "short",
    day: "numeric",
  });
}
function utc(value: string) {
  return new Date(value).toLocaleString(undefined, {
    timeZone: "UTC",
    timeZoneName: "short",
  });
}
