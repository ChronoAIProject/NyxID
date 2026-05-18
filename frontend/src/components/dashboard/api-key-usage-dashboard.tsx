import { Link, useNavigate } from "@tanstack/react-router";
import { Area, AreaChart } from "recharts";
import { useApiKeysUsage } from "@/hooks/use-api-keys";
import { ErrorBanner } from "@/components/shared/error-banner";
import { formatRelativeTime } from "@/lib/utils";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
} from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { Activity } from "lucide-react";
import {
  type ChartConfig,
  ChartContainer,
} from "@/components/ui/chart";

const miniChartConfig = {
  requests: { label: "Requests", color: "#A672FB" },
} satisfies ChartConfig;

function MiniLineChart({
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

  const chartData = buckets.map((b) => ({
    date: b.date,
    requests: b.request_count,
  }));

  return (
    <ChartContainer config={miniChartConfig} className="h-[48px] w-full aspect-auto" style={{ pointerEvents: "none" }}>
      <AreaChart data={chartData} margin={{ top: 0, right: 0, bottom: 0, left: 0 }}>
        <defs>
          <linearGradient id="miniFillRequests" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#A672FB" stopOpacity={0.25} />
            <stop offset="100%" stopColor="#A672FB" stopOpacity={0} />
          </linearGradient>
        </defs>
        <Area
          dataKey="requests"
          type="natural"
          stroke="#A672FB"
          strokeWidth={1.5}
          fill="url(#miniFillRequests)"
          dot={false}
        />
      </AreaChart>
    </ChartContainer>
  );
}

export function ApiKeyUsageDashboard({
  viewMode = "grid",
}: {
  readonly viewMode?: "grid" | "table";
} = {}) {
  const navigate = useNavigate();
  const { data, isLoading, error, refetch } = useApiKeysUsage(7);

  if (isLoading) {
    return (
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <Activity className="h-4 w-4 text-muted-foreground" />
          <h3 className="text-[13px] font-semibold text-foreground">Agent Activity</h3>
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-44 rounded-xl" />
          ))}
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <Activity className="h-4 w-4 text-muted-foreground" />
          <h3 className="text-[13px] font-semibold text-foreground">Agent Activity</h3>
        </div>
        <ErrorBanner message="Failed to load agent activity." onRetry={refetch} />
      </div>
    );
  }

  if (!data || data.length === 0) {
    return null;
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <Activity className="h-4 w-4 text-muted-foreground" />
        <h3 className="text-[13px] font-semibold text-foreground">Agent Activity</h3>
      </div>

      {viewMode === "table" ? (
        <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow className="border-border/50 hover:bg-transparent">
                <TableHead className="w-[25%]">Agent</TableHead>
                <TableHead className="w-[15%]">Platform</TableHead>
                <TableHead className="w-[15%]">Requests</TableHead>
                <TableHead className="w-[30%]">Trend</TableHead>
                <TableHead className="w-[15%]">Last Used</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((usage) => (
                <TableRow
                  key={usage.api_key_id}
                  className="border-border/30 cursor-pointer hover:bg-white/[0.03]"
                  onClick={() => void navigate({ to: "/keys/api-key/$keyId", params: { keyId: usage.api_key_id } })}
                >
                  <TableCell>
                    <p className="truncate font-medium text-foreground">
                      {usage.api_key_name}
                    </p>
                  </TableCell>
                  <TableCell>
                    <Badge variant="secondary">
                      {usage.platform ?? "agent"}
                    </Badge>
                  </TableCell>
                  <TableCell className="text-foreground font-semibold">
                    {usage.request_count}
                  </TableCell>
                  <TableCell>
                    <MiniLineChart buckets={usage.daily_buckets} />
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {usage.last_used_at
                      ? formatRelativeTime(usage.last_used_at)
                      : "never"}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {data.map((usage) => (
            <Link
              key={usage.api_key_id}
              to="/keys/api-key/$keyId"
              params={{ keyId: usage.api_key_id }}
            >
              <Card className="h-full transition-colors duration-300 hover:border-white/[0.15] hover:bg-accent/30">
                <CardContent className="space-y-3 p-4">
                  <div className="flex items-center justify-between gap-2">
                    <p className="truncate text-[12px] font-medium text-foreground">
                      {usage.api_key_name}
                    </p>
                    <Badge variant={usage.platform ? "secondary" : "secondary"}>
                      {usage.platform ?? "agent"}
                    </Badge>
                  </div>

                  <div className="flex items-baseline gap-1.5">
                    <span className="text-[24px] font-bold leading-none text-foreground">
                      {usage.request_count}
                    </span>
                    <span className="text-[11px] text-muted-foreground">requests</span>
                  </div>

                  <MiniLineChart buckets={usage.daily_buckets} />

                  <p className="text-[11px] text-muted-foreground">
                    Last used{" "}
                    {usage.last_used_at
                      ? formatRelativeTime(usage.last_used_at)
                      : "never"}
                  </p>
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
