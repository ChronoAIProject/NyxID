import { useState } from "react";
import { Area, AreaChart, XAxis, YAxis } from "recharts";
import { useApiKeyUsage } from "@/hooks/use-api-keys";
import { ErrorBanner } from "@/components/shared/error-banner";
import { formatRelativeTime } from "@/lib/utils";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Activity } from "lucide-react";
import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const chartConfig = {
  requests: { label: "Requests", color: "#A672FB" },
  errors: { label: "Errors", color: "var(--color-destructive)" },
} satisfies ChartConfig;

function ActivityChart({
  buckets,
}: {
  readonly buckets: readonly {
    readonly date: string;
    readonly request_count: number;
    readonly error_count: number;
  }[];
}) {
  if (buckets.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        No usage in the selected window.
      </p>
    );
  }

  const chartData = buckets.map((b) => ({
    date: b.date,
    requests: b.request_count,
    errors: b.error_count,
  }));

  return (
    <ChartContainer config={chartConfig} className="h-[120px] w-full aspect-auto">
      <AreaChart data={chartData} margin={{ top: 4, right: 8, bottom: 0, left: 8 }}>
        <defs>
          <linearGradient id="fillRequests" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#A672FB" stopOpacity={0.25} />
            <stop offset="100%" stopColor="#A672FB" stopOpacity={0} />
          </linearGradient>
          <linearGradient id="fillErrors" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--color-destructive)" stopOpacity={0.2} />
            <stop offset="100%" stopColor="var(--color-destructive)" stopOpacity={0} />
          </linearGradient>
        </defs>
        <XAxis
          dataKey="date"
          tickLine={false}
          axisLine={false}
          tickFormatter={(value: string) =>
            new Date(value + "T00:00:00").toLocaleDateString("en-US", {
              month: "short",
              day: "numeric",
            })
          }
          tick={{ fontSize: 10 }}
          interval="preserveStartEnd"
        />
        <YAxis hide />
        <ChartTooltip
          content={
            <ChartTooltipContent
              labelFormatter={(value) =>
                new Date(value + "T00:00:00").toLocaleDateString("en-US", {
                  month: "short",
                  day: "numeric",
                })
              }
            />
          }
        />
        <Area
          dataKey="requests"
          type="natural"
          stroke="#A672FB"
          strokeWidth={1.5}
          fill="url(#fillRequests)"
          dot={false}
        />
        <Area
          dataKey="errors"
          type="natural"
          stroke="var(--color-destructive)"
          strokeWidth={1.5}
          fill="url(#fillErrors)"
          dot={false}
        />
      </AreaChart>
    </ChartContainer>
  );
}

export function UsageStatsCard({
  keyId,
}: {
  readonly keyId: string;
}) {
  const [days, setDays] = useState(7);
  const { data, isLoading, error, refetch } = useApiKeyUsage(keyId, days);

  return (
    <Card className="md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Activity className="h-4 w-4 text-primary" />
            <CardTitle className="text-[15px]">Usage</CardTitle>
          </div>
          <Select value={String(days)} onValueChange={(v) => setDays(Number(v))}>
            <SelectTrigger className="h-8 w-[160px] text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="7">Last 7 Days</SelectItem>
              <SelectItem value="30">Last 30 Days</SelectItem>
              <SelectItem value="90">Last 3 Months</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <CardDescription>
          Agent-attributed requests, provider-reported tokens, and reported cost.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isLoading ? (
          <div className="space-y-3">
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-24 w-full" />
          </div>
        ) : error || !data ? (
          <ErrorBanner message="Failed to load usage stats." onRetry={refetch} />
        ) : (
          <>
            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              <div className="rounded-lg border border-border p-3">
                <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
                  Requests
                </p>
                <p className="mt-1 text-lg font-semibold">{data.request_count}</p>
              </div>
              <div className="rounded-lg border border-border p-3">
                <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
                  Successes
                </p>
                <p className="mt-1 text-lg font-semibold">{data.success_count}</p>
              </div>
              <div className="rounded-lg border border-border p-3">
                <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
                  Errors
                </p>
                <p className="mt-1 text-lg font-semibold">{data.error_count}</p>
              </div>
              <div className="rounded-lg border border-border p-3">
                <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
                  Error Rate
                </p>
                <p className="mt-1 text-lg font-semibold">
                  {(data.error_rate * 100).toFixed(1)}%
                </p>
              </div>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <p className="text-xs font-medium text-foreground">Daily Activity</p>
                <span className="text-xs text-muted-foreground">
                  Last used{" "}
                  {data.last_used_at ? formatRelativeTime(data.last_used_at) : "never"}
                </span>
              </div>
              <ActivityChart buckets={data.daily_buckets} />
            </div>

            <div className="space-y-2">
              <p className="text-xs font-medium text-foreground">Top Services</p>
              {data.top_services.length > 0 ? (
                <div className="flex flex-wrap gap-2">
                  {data.top_services.map((service) => (
                    <Badge key={`${service.service_slug}-${service.request_count}`} variant="secondary">
                      {service.service_label} ({service.request_count})
                    </Badge>
                  ))}
                </div>
              ) : (
                <p className="text-xs text-muted-foreground">
                  No attributed service traffic yet.
                </p>
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}
