import { useState } from "react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  modelLabel,
  seriesColor,
  costFields,
  tokenFields,
  modelMeasureLabels,
  modelDailyPoints,
  type Measure,
} from "@/lib/nyxbot-model-usage";
import type { NyxbotModelUsage } from "@/schemas/nyxbot-model-usage";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { chartTick } from "@/components/billing-analytics/chart-style";
import { Card } from "@/components/ui/card";
function Trend({
  data,
  measure,
  title,
}: {
  data: NyxbotModelUsage;
  measure: Measure;
  title: string;
}) {
  const points = modelDailyPoints(data, measure);
  return (
    <div className="min-w-0 space-y-2">
      <h3 className="text-12 font-semibold">{title} · Daily UTC</h3>
      <div
        className="analytics-plot h-56"
        role="img"
        aria-label={`${title} by model. Exact daily values in the table below.`}
      >
        <ResponsiveContainer
          width="100%"
          height="100%"
          minWidth={1}
          initialDimension={{ width: 640, height: 224 }}
        >
          <LineChart
            data={points}
            accessibilityLayer
            margin={{ top: 8, right: 16, left: 0, bottom: 4 }}
          >
            <CartesianGrid
              vertical={false}
              stroke="var(--analytics-grid)"
              strokeDasharray="4 6"
            />
            <XAxis
              dataKey="day"
              tickFormatter={(day: string) => day.slice(5)}
              tick={chartTick}
              minTickGap={35}
            />
            <YAxis
              width={60}
              tickFormatter={(v: number) =>
                v.toLocaleString(undefined, {
                  notation: "compact",
                  maximumSignificantDigits: 3,
                })
              }
              tick={chartTick}
            />
            <Tooltip
              content={({ active, label: day, payload }) =>
                active && payload?.length ? (
                  <div className="max-h-72 max-w-80 overflow-auto rounded-lg border border-border bg-popover p-3 text-11 shadow-lg">
                    <p className="mb-1 font-semibold">
                      {day} UTC · {modelMeasureLabels[measure]}
                    </p>
                    {data.models.map((model, index) => (
                      <p key={index} className="break-words">
                        {modelLabel(model)}:{" "}
                        <span className="font-mono">
                          {payload[0]?.payload[`exact${index}`]}
                        </span>
                      </p>
                    ))}
                  </div>
                ) : null
              }
            />
            {data.models.map((model, index) => (
              <Line
                key={index}
                name={modelLabel(model)}
                dataKey={`s${index}`}
                stroke={seriesColor(index, model.is_other)}
                strokeWidth={1.8}
                strokeDasharray={model.is_other ? "4 3" : undefined}
                dot={points.length === 1}
                connectNulls={false}
                isAnimationActive={false}
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </div>
      <details className="text-11">
        <summary className="cursor-pointer text-muted-foreground">
          Exact daily {modelMeasureLabels[measure].toLowerCase()}
        </summary>
        <div
          className="mt-2 max-h-80 overflow-auto"
          role="region"
          aria-label={`Exact daily ${modelMeasureLabels[measure].toLowerCase()}`}
          tabIndex={0}
        >
          <table className="w-full text-left">
            <caption className="sr-only">{title} daily exact values</caption>
            <thead>
              <tr>
                <th scope="col" className="p-2">
                  UTC date
                </th>
                {data.models.map((model, i) => (
                  <th key={i} scope="col" className="min-w-36 p-2">
                    {modelLabel(model)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {points.map((point) => (
                <tr key={point.day}>
                  <th scope="row" className="whitespace-nowrap p-2 font-normal">
                    {point.day}
                  </th>
                  {data.models.map((_, i) => (
                    <td
                      key={i}
                      className="whitespace-nowrap p-2 font-mono tabular-nums"
                    >
                      {point[`exact${i}`]}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </div>
  );
}
export function ModelTrends({
  data,
  kind,
}: {
  data: NyxbotModelUsage;
  kind: "cost" | "traffic";
}) {
  const [cost, setCost] = useState<(typeof costFields)[number]>("gross_cost");
  const [tokens, setTokens] =
    useState<(typeof tokenFields)[number]>("total_tokens");
  const availableTokens = tokenFields.filter(
    (f) =>
      (!f.startsWith("audio_") && !f.startsWith("image_")) ||
      data.models.some((m) => m.usage[f]),
  );
  const effectiveTokens = availableTokens.includes(tokens)
    ? tokens
    : "total_tokens";
  return (
    <Card className="min-w-0 space-y-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-11 text-muted-foreground">
          Full-period Top 20 models + Other · days without usage show zero ·
          gaps mean cost is unavailable
        </p>
        <div className="flex items-center gap-2 text-11">
          <span>{kind === "cost" ? "Cost measure" : "Token measure"}</span>
          <Select
            value={kind === "cost" ? cost : effectiveTokens}
            onValueChange={(value) =>
              kind === "cost"
                ? setCost(value as typeof cost)
                : setTokens(value as typeof tokens)
            }
          >
            <SelectTrigger
              className="w-44"
              aria-label={kind === "cost" ? "Cost measure" : "Token measure"}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(kind === "cost" ? costFields : availableTokens).map((field) => (
                <SelectItem key={field} value={field}>
                  {modelMeasureLabels[field]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-11">
        {data.models.map((m, i) => (
          <span key={i} className="min-w-0 break-all">
            <svg
              aria-hidden="true"
              width="20"
              height="8"
              className="mr-1 inline-block"
            >
              <line
                x1="0"
                y1="4"
                x2="20"
                y2="4"
                stroke={seriesColor(i, m.is_other)}
                strokeWidth="2"
                strokeDasharray={m.is_other ? "4 3" : undefined}
              />
            </svg>
            {modelLabel(m)}
          </span>
        ))}
      </div>
      {kind === "cost" ? (
        <Trend data={data} measure={cost} title={modelMeasureLabels[cost]} />
      ) : (
        <div className="grid min-w-0 gap-6 xl:grid-cols-2">
          <Trend data={data} measure="requests" title="Requests" />
          <Trend
            data={data}
            measure={effectiveTokens}
            title={modelMeasureLabels[effectiveTokens]}
          />
        </div>
      )}
    </Card>
  );
}
