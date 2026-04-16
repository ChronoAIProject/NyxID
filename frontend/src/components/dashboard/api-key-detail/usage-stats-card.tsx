import { useApiKeyUsage } from "@/hooks/use-api-keys";
import { formatRelativeTime } from "@/lib/utils";
import { formatBucketLabel } from "@/lib/usage-bucket";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Activity } from "lucide-react";

function ActivityBars({
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

  const maxCount = Math.max(...buckets.map((bucket) => bucket.request_count), 1);

  return (
    <div className="flex items-end gap-2">
      {buckets.map((bucket) => {
        const label = formatBucketLabel(bucket);
        return (
          <Tooltip key={bucket.date}>
            <TooltipTrigger asChild>
              <div
                tabIndex={0}
                role="img"
                aria-label={label}
                className="flex min-w-0 flex-1 cursor-default flex-col items-center gap-1 rounded outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="h-3.5 text-[10px] font-medium leading-none text-foreground tabular-nums">
                  {bucket.request_count > 0 ? bucket.request_count : ""}
                </span>
                <div className="flex h-16 w-full items-end rounded bg-muted/40 px-1">
                  <div
                    className="w-full rounded-sm bg-primary/80"
                    style={{
                      height: `${Math.max((bucket.request_count / maxCount) * 100, bucket.request_count > 0 ? 10 : 2)}%`,
                    }}
                  />
                </div>
                <span className="text-[10px] text-muted-foreground">
                  {bucket.date.slice(5)}
                </span>
              </div>
            </TooltipTrigger>
            <TooltipContent side="top">{label}</TooltipContent>
          </Tooltip>
        );
      })}
    </div>
  );
}

export function UsageStatsCard({
  keyId,
}: {
  readonly keyId: string;
}) {
  const { data, isLoading, error } = useApiKeyUsage(keyId, 7);

  return (
    <Card className="md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Activity className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">Usage</CardTitle>
        </div>
        <CardDescription>
          Agent-attributed requests, provider-reported tokens, and reported cost for the last 7 days.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isLoading ? (
          <div className="space-y-3">
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-24 w-full" />
          </div>
        ) : error || !data ? (
          <p className="text-xs text-muted-foreground">
            Failed to load usage stats.
          </p>
        ) : (
          <>
            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              <div className="rounded-lg border border-border p-3">
                <p className="text-[11px] uppercase tracking-wide text-muted-foreground">
                  Requests
                </p>
                <p className="mt-1 text-lg font-semibold">{data.request_count}</p>
              </div>
              <div className="rounded-lg border border-border p-3">
                <p className="text-[11px] uppercase tracking-wide text-muted-foreground">
                  Successes
                </p>
                <p className="mt-1 text-lg font-semibold">{data.success_count}</p>
              </div>
              <div className="rounded-lg border border-border p-3">
                <p className="text-[11px] uppercase tracking-wide text-muted-foreground">
                  Errors
                </p>
                <p className="mt-1 text-lg font-semibold">{data.error_count}</p>
              </div>
              <div className="rounded-lg border border-border p-3">
                <p className="text-[11px] uppercase tracking-wide text-muted-foreground">
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
              <ActivityBars buckets={data.daily_buckets} />
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
