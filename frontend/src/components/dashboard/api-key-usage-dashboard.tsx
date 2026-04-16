import { Link } from "@tanstack/react-router";
import { useApiKeysUsage } from "@/hooks/use-api-keys";
import { formatRelativeTime } from "@/lib/utils";
import { formatBucketLabel } from "@/lib/usage-bucket";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Activity } from "lucide-react";

function UsageBars({
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
        No recent activity.
      </p>
    );
  }

  const maxCount = Math.max(...buckets.map((bucket) => bucket.request_count), 1);

  return (
    <div className="flex items-end gap-1.5">
      {buckets.map((bucket) => {
        const label = formatBucketLabel(bucket);
        return (
          <Tooltip key={bucket.date}>
            <TooltipTrigger asChild>
              <div
                role="img"
                aria-label={label}
                className="flex min-w-0 flex-1 flex-col items-center gap-0.5 rounded outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="h-3.5 text-[10px] font-medium leading-none text-foreground tabular-nums">
                  {bucket.request_count > 0 ? bucket.request_count : ""}
                </span>
                <div className="flex h-10 w-full items-end rounded bg-muted/40 px-0.5">
                  <div
                    className="w-full rounded-sm bg-primary/80"
                    style={{
                      height: `${Math.max((bucket.request_count / maxCount) * 100, bucket.request_count > 0 ? 10 : 2)}%`,
                    }}
                  />
                </div>
                <span className="text-[10px] text-muted-foreground">
                  {bucket.date.slice(8)}
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

export function ApiKeyUsageDashboard() {
  const { data, isLoading, error } = useApiKeysUsage(7);

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Activity className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">Agent Usage Dashboard</CardTitle>
        </div>
        <CardDescription>
          Request volume, provider-reported tokens, reported cost, and top services for the last 7 days.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 3 }, (_, index) => (
              <Skeleton key={index} className="h-44 w-full" />
            ))}
          </div>
        ) : error ? (
          <p className="text-sm text-muted-foreground">
            Failed to load agent usage data.
          </p>
        ) : data && data.length > 0 ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {data.map((usage) => (
              <Link
                key={usage.api_key_id}
                to="/keys/api-key/$keyId"
                params={{ keyId: usage.api_key_id }}
              >
                <Card className="h-full transition-colors hover:border-primary/30 hover:bg-accent/30">
                  <CardContent className="space-y-4 p-5">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="truncate text-sm font-medium text-foreground">
                          {usage.api_key_name}
                        </p>
                        <p className="text-xs text-muted-foreground">
                          Last used{" "}
                          {usage.last_used_at
                            ? formatRelativeTime(usage.last_used_at)
                            : "never"}
                        </p>
                      </div>
                      <Badge variant={usage.platform ? "secondary" : "outline"}>
                        {usage.platform ?? "agent"}
                      </Badge>
                    </div>

                    <div className="grid grid-cols-3 gap-2 text-xs">
                      <div className="rounded-lg border border-border p-2">
                        <p className="text-muted-foreground">Requests</p>
                        <p className="mt-1 text-sm font-semibold">{usage.request_count}</p>
                      </div>
                      <div className="rounded-lg border border-border p-2">
                        <p className="text-muted-foreground">Errors</p>
                        <p className="mt-1 text-sm font-semibold">{usage.error_count}</p>
                      </div>
                      <div className="rounded-lg border border-border p-2">
                        <p className="text-muted-foreground">Error Rate</p>
                        <p className="mt-1 text-sm font-semibold">
                          {(usage.error_rate * 100).toFixed(1)}%
                        </p>
                      </div>
                    </div>

                    <UsageBars buckets={usage.daily_buckets} />

                    <div className="space-y-1.5">
                      <p className="text-xs font-medium text-foreground">Top Services</p>
                      {usage.top_services.length > 0 ? (
                        <div className="flex flex-wrap gap-1.5">
                          {usage.top_services.slice(0, 3).map((service) => (
                            <Badge
                              key={`${usage.api_key_id}-${service.service_slug}`}
                              variant="secondary"
                              className="text-[11px]"
                            >
                              {service.service_label} ({service.request_count})
                            </Badge>
                          ))}
                        </div>
                      ) : (
                        <p className="text-xs text-muted-foreground">
                          No service usage yet.
                        </p>
                      )}
                    </div>
                  </CardContent>
                </Card>
              </Link>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            No agent usage recorded yet.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
