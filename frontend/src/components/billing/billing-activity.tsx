import { useState } from "react";
import { Line, LineChart, XAxis, YAxis } from "recharts";
import { useApiKeysUsage } from "@/hooks/use-api-keys";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { AnalyticsSelect } from "@/components/billing-analytics/filter-picker";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Skeleton } from "@/components/ui/skeleton";
import { number } from "@/lib/billing-display";
import type { ApiKeyUsage } from "@/types/api";

const chartConfig = {
  requests: { label: "Requests", color: "var(--billing-chart-accent)" },
  errors: { label: "Errors", color: "var(--color-destructive)" },
} satisfies ChartConfig;

function dateLabel(date: string) {
  return new Date(`${date}T00:00:00Z`).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
}

function agentActivityPoints(agents: readonly ApiKeyUsage[]) {
  const points = new Map<
    string,
    { date: string; requests: number; errors: number }
  >();
  for (const agent of agents) {
    for (const bucket of agent.daily_buckets) {
      const point = points.get(bucket.date) ?? {
        date: bucket.date,
        requests: 0,
        errors: 0,
      };
      point.requests += bucket.request_count;
      point.errors += bucket.error_count;
      points.set(bucket.date, point);
    }
  }
  return [...points.values()].sort((a, b) => a.date.localeCompare(b.date));
}

export function BillingActivity() {
  const [days, setDays] = useState(7);
  const [agentId, setAgentId] = useState("all");
  const activity = useApiKeysUsage(days);
  const agents = activity.data ?? [];
  const selected = agents.find((agent) => agent.api_key_id === agentId);
  const points = agentActivityPoints(selected ? [selected] : agents);
  const requests = points.reduce((sum, point) => sum + point.requests, 0);
  const errors = points.reduce((sum, point) => sum + point.errors, 0);
  return (
    <Card className="billing-activity">
      <CardHeader className="usage-explorer-heading">
        <div>
          <CardTitle>Daily agent activity</CardTitle>
          <CardDescription>
            Personal agent requests across all services. Billing filters apply
            to the breakdown below.
          </CardDescription>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <AnalyticsSelect
            label="Activity time range"
            value={String(days)}
            onChange={(value) => setDays(Number(value))}
            options={[
              { value: "7", label: "Last 7 days" },
              { value: "30", label: "Last 30 days" },
            ]}
          />
          <AnalyticsSelect
            label="Activity agent"
            value={selected?.api_key_id ?? "all"}
            onChange={setAgentId}
            options={[
              { value: "all", label: "All personal agents" },
              ...agents.map((agent) => ({
                value: agent.api_key_id,
                label: agent.api_key_name,
              })),
            ]}
          />
        </div>
      </CardHeader>
      <CardContent>
        {activity.isLoading ? (
          <Skeleton className="h-[160px] w-full" />
        ) : activity.error ? (
          <ErrorBanner
            message="Could not load daily agent activity."
            onRetry={() => void activity.refetch()}
          />
        ) : points.length === 0 ? (
          <p className="text-12 text-muted-foreground">
            No agent activity in this window.
          </p>
        ) : (
          <>
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3 text-11">
              <p className="font-mono tabular-nums">
                {number(requests)} requests · {number(errors)} errors
              </p>
              <div className="flex gap-4 text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <span
                    className="h-0.5 w-3"
                    style={{ background: chartConfig.requests.color }}
                  />
                  Requests
                </span>
                <span className="inline-flex items-center gap-1.5">
                  <span className="h-0.5 w-3 bg-destructive" />
                  Errors
                </span>
              </div>
            </div>
            <div
              role="img"
              aria-label={`Daily activity: ${number(requests)} requests and ${number(errors)} errors across ${points.length} UTC days`}
            >
              <ChartContainer
                config={chartConfig}
                className="h-[160px] w-full aspect-auto"
              >
                <LineChart
                  data={points}
                  margin={{ top: 8, right: 8, bottom: 0, left: 8 }}
                >
                  <XAxis
                    dataKey="date"
                    tickLine={false}
                    axisLine={false}
                    tickFormatter={dateLabel}
                    tick={{
                      fontSize: 10,
                      fill: "var(--color-muted-foreground)",
                    }}
                    interval="preserveStartEnd"
                    minTickGap={24}
                  />
                  <YAxis hide domain={[0, "auto"]} />
                  <ChartTooltip
                    content={
                      <ChartTooltipContent
                        labelFormatter={(value) =>
                          `${dateLabel(String(value))} · UTC`
                        }
                      />
                    }
                  />
                  <Line
                    dataKey="requests"
                    type="monotone"
                    stroke={chartConfig.requests.color}
                    strokeWidth={1.5}
                    dot={points.length === 1}
                    isAnimationActive={false}
                  />
                  <Line
                    dataKey="errors"
                    type="monotone"
                    stroke="var(--color-destructive)"
                    strokeWidth={1.5}
                    dot={points.length === 1}
                    isAnimationActive={false}
                  />
                </LineChart>
              </ChartContainer>
            </div>
            <details className="mt-3 text-11">
              <summary className="text-muted-foreground">
                View daily values
              </summary>
              <div className="mt-3 max-h-64 overflow-auto">
                <table className="w-full text-left tabular-nums">
                  <caption className="sr-only">
                    Agent activity per UTC day
                  </caption>
                  <thead>
                    <tr>
                      <th className="py-2 font-medium">Date (UTC)</th>
                      <th className="py-2 text-right font-medium">Requests</th>
                      <th className="py-2 text-right font-medium">Errors</th>
                    </tr>
                  </thead>
                  <tbody>
                    {points.map((point) => (
                      <tr key={point.date} className="border-t border-border">
                        <td className="py-2">{point.date}</td>
                        <td className="py-2 text-right font-mono">
                          {number(point.requests)}
                        </td>
                        <td className="py-2 text-right font-mono">
                          {number(point.errors)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </details>
          </>
        )}
      </CardContent>
    </Card>
  );
}
