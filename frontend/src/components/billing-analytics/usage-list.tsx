import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { ChevronDown, ChevronLeft, ChevronRight } from "lucide-react";
import type { AnalyticsFilters } from "@/schemas/usage-analytics";
import { useAdminUsage, type AdminUsageParams } from "@/hooks/use-admin-usage";
import { credentialClassLabel } from "@/lib/billing-units";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ArticleIcon } from "@/components/icons/empty-state";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { DataTableBadgeCell } from "@/components/data-table/data-table-columns";
import { formatNumber, formatEstimatedCredits } from "@/lib/billing-format";
import { BILLING_METRICS, metricLabel } from "@/schemas/billing-metrics";
import {
  normalizeAdminUsageSearch,
  usageRangeError,
  USAGE_SORTS,
} from "@/schemas/admin-usage";
import type {
  AdminUsageIdentity,
  AdminUsageRanking,
  AdminUsageSearch,
  AdminUsageService,
  AdminUsageStats,
} from "@/types/admin";

const SORT_LABELS: Record<AdminUsageSearch["sort"], string> = {
  requests: "Requests",
  quantity: "Metric quantity",
  cost: "Gross cost",
  total_tokens: "Total tokens",
  prompt_tokens: "Input tokens",
  completion_tokens: "Output tokens",
  cached_tokens: "Cache-read tokens",
  cache_creation_tokens: "Cache-write tokens",
};

function Choice({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger
        aria-label={label}
        className="w-full sm:w-auto sm:min-w-40"
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
function Identity({ user }: { user: AdminUsageIdentity }) {
  return (
    <div className="min-w-0">
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="text-[12px] font-medium">{user.display_name}</span>
        </TooltipTrigger>
        <TooltipContent>{user.id}</TooltipContent>
      </Tooltip>
      {user.email && (
        <div className="break-all text-[11px] text-muted-foreground">
          {user.email}
        </div>
      )}
      {user.user_type === "org" && (
        <Badge variant="secondary">Organization</Badge>
      )}
    </div>
  );
}
function Quantities({ usage }: { usage: AdminUsageStats }) {
  return (
    <div className="space-y-1 font-mono text-[11px] tabular-nums">
      {Object.entries(usage.quantities).map(([metric, quantity]) => (
        <div key={metric}>
          {formatNumber(quantity)} {metricLabel(metric)}
        </div>
      ))}
      {usage.total_tokens > 0 && (
        <div className="text-muted-foreground">
          {formatNumber(usage.total_tokens)} total tokens · in{" "}
          {formatNumber(usage.prompt_tokens)} · out{" "}
          {formatNumber(usage.completion_tokens)}
          <br />
          cache read {formatNumber(usage.cached_tokens)} · cache write{" "}
          {formatNumber(usage.cache_creation_tokens)}
        </div>
      )}
    </div>
  );
}
function Cost({ usage }: { usage: AdminUsageStats }) {
  return (
    <div className="space-y-1 font-mono text-[11px] tabular-nums">
      <span>{formatEstimatedCredits(usage.gross_cost_micros)}</span>
      <div className="text-muted-foreground">
        Wallet {formatEstimatedCredits(usage.wallet_cost_micros)}
        <br />
        Grants {formatEstimatedCredits(usage.grant_cost_micros)}
        <br />
        Allowance {formatEstimatedCredits(usage.allowance_cost_micros)}
      </div>
      {usage.unknown_cost_events > 0 && (
        <Badge variant="warning">Partial estimate</Badge>
      )}
    </div>
  );
}
function ServiceName({
  service,
}: {
  service: Pick<AdminUsageService, "service_name" | "service_slug">;
}) {
  return (
    <div className="text-[12px] font-medium">
      {service.service_name}
      {service.service_slug && (
        <div className="text-[11px] font-normal text-muted-foreground">
          {service.service_slug}
        </div>
      )}
    </div>
  );
}
function StatsTable({
  rows,
  firstHeading = "Service",
}: {
  rows: { key: string; label: ReactNode; usage: AdminUsageStats }[];
  firstHeading?: string;
}) {
  return (
    <>
      <div className="hidden overflow-hidden rounded-lg border border-border bg-card md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{firstHeading}</TableHead>
              <TableHead>Requests</TableHead>
              <TableHead>Usage</TableHead>
              <TableHead>Gross cost</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.key}>
                <TableCell>{row.label}</TableCell>
                <TableCell className="font-mono tabular-nums">
                  {formatNumber(row.usage.requests)}
                </TableCell>
                <TableCell>
                  <Quantities usage={row.usage} />
                </TableCell>
                <TableCell>
                  <Cost usage={row.usage} />
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      <div className="flex flex-col gap-3 md:hidden">
        {rows.map((row) => (
          <div
            key={row.key}
            className="space-y-3 rounded-lg border border-border bg-card p-4"
          >
            {row.label}
            <div className="font-mono text-[11px]">
              {formatNumber(row.usage.requests)} requests
            </div>
            <Quantities usage={row.usage} />
            <Cost usage={row.usage} />
          </div>
        ))}
      </div>
    </>
  );
}
function ServiceTable({ services }: { services: AdminUsageService[] }) {
  return (
    <StatsTable
      rows={services.map((service) => ({
        key: `${service.service_id}:${service.service_slug}`,
        label: (
          <div className="space-y-2">
            <ServiceName service={service} />
            <span className="text-[11px] text-muted-foreground">
              {formatNumber(service.unique_users)} users
            </span>
            <DataTableBadgeCell>
              {service.by_credential_class.map((lane) => (
                <Badge key={lane.credential_class} variant="secondary">
                  {credentialClassLabel(lane.credential_class)}:{" "}
                  <span className="font-mono">
                    {formatNumber(lane.requests)}
                  </span>
                </Badge>
              ))}
            </DataTableBadgeCell>
          </div>
        ),
        usage: service,
      }))}
    />
  );
}
function UserServices({
  user,
  search,
}: {
  user: AdminUsageIdentity;
  search: AdminUsageParams;
}) {
  const usage = useAdminUsage({
    ...search,
    user: user.id,
    service: undefined,
    page: 1,
  });
  return (
    <div className="space-y-3 p-3">
      <p className="text-[12px] font-medium">
        Services for {user.display_name}
      </p>
      {usage.isPending ? (
        <Skeleton className="h-20" />
      ) : usage.isError ? (
        <ErrorBanner
          message="Could not load user breakdown."
          onRetry={() => void usage.refetch()}
        />
      ) : (
        usage.data && <ServiceTable services={usage.data.by_service} />
      )}
    </div>
  );
}
function RankingTable({
  rows,
  search,
}: {
  rows: AdminUsageRanking[];
  search: AdminUsageParams;
}) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const rowKey = (row: AdminUsageRanking) =>
    `${row.user.id}:${row.billing_owner?.id}:${row.service_id}:${row.service_slug}`;
  const person = (row: AdminUsageRanking) => (
    <div className="space-y-2">
      <Identity user={row.user} />
      {row.billing_owner && (
        <div className="border-l border-border pl-2">
          <span className="text-[10px] text-muted-foreground">Billed to</span>
          <Identity user={row.billing_owner} />
        </div>
      )}
      {!search.service && (
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={expanded === rowKey(row)}
          onClick={() =>
            setExpanded(expanded === rowKey(row) ? null : rowKey(row))
          }
        >
          Services <ChevronDown className="size-3" />
        </Button>
      )}
    </div>
  );
  return (
    <>
      <div className="hidden overflow-hidden rounded-lg border border-border bg-card md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>User</TableHead>
              <TableHead>Service</TableHead>
              <TableHead>Usage</TableHead>
              <TableHead>Requests</TableHead>
              <TableHead>Gross cost</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <Fragment key={rowKey(row)}>
                <TableRow>
                  <TableCell>{person(row)}</TableCell>
                  <TableCell>
                    <ServiceName service={row} />
                  </TableCell>
                  <TableCell>
                    <Quantities usage={row} />
                  </TableCell>
                  <TableCell className="font-mono tabular-nums">
                    {formatNumber(row.requests)}
                  </TableCell>
                  <TableCell>
                    <Cost usage={row} />
                  </TableCell>
                </TableRow>
                {expanded === rowKey(row) && (
                  <TableRow>
                    <TableCell colSpan={5}>
                      <UserServices user={row.user} search={search} />
                    </TableCell>
                  </TableRow>
                )}
              </Fragment>
            ))}
          </TableBody>
        </Table>
      </div>
      <div className="flex flex-col gap-3 md:hidden">
        {rows.map((row) => (
          <div
            key={rowKey(row)}
            className="space-y-3 rounded-lg border border-border bg-card p-4"
          >
            {person(row)}
            <ServiceName service={row} />
            <p className="font-mono text-[11px]">
              {formatNumber(row.requests)} requests
            </p>
            <Quantities usage={row} />
            <Cost usage={row} />
            {expanded === rowKey(row) && (
              <UserServices user={row.user} search={search} />
            )}
          </div>
        ))}
      </div>
    </>
  );
}
export function AdminUsageList({ filters }: { filters: AnalyticsFilters }) {
  const routeSearch = useSearch({ from: "/dashboard/admin/usage" });
  const search = normalizeAdminUsageSearch(routeSearch);
  const navigate = useNavigate();
  const filterKey = JSON.stringify(filters);
  const previousFilters = useRef(filterKey);
  useEffect(() => {
    if (previousFilters.current !== filterKey) {
      previousFilters.current = filterKey;
      if (search.page !== 1)
        void navigate({
          to: "/admin/usage",
          search: { ...routeSearch, tab: "list", page: 1 },
          replace: true,
        });
    }
  }, [filterKey, navigate, routeSearch, search.page]);
  const effectiveSearch: AdminUsageParams = {
    ...search,
    page: search.page,
    period: filters.period,
    from: filters.from ?? undefined,
    to: filters.to ?? undefined,
    user: undefined,
    service: undefined,
    services: filters.services,
    actors: filters.actors,
    owners: filters.owners,
  };
  const usage = useAdminUsage(effectiveSearch);
  const data = usage.data;
  const change = (patch: Partial<AdminUsageSearch>) =>
    void navigate({
      to: "/admin/usage",
      search: { ...routeSearch, tab: "list", page: 1, ...patch },
    });
  const rangeError =
    effectiveSearch.period === "custom"
      ? usageRangeError(effectiveSearch.from, effectiveSearch.to)
      : null;
  const pages = Math.max(
    1,
    Math.ceil((data?.ranking_total ?? 0) / search.per_page),
  );
  // Expansion reads the exact response window so an interaction cannot shift
  // a rolling boundary between the summary and the user's detail.
  const detailSearch: AdminUsageParams = data
    ? {
        ...effectiveSearch,
        period: "custom",
        from: data.window.from,
        to: data.window.to,
      }
    : effectiveSearch;
  return (
    <div className="space-y-6">
      {rangeError ? (
        <p role="alert" className="text-[12px] text-destructive">
          {rangeError}
        </p>
      ) : usage.isError ? (
        <ErrorBanner
          message={
            usage.error instanceof Error
              ? usage.error.message
              : "Failed to load usage."
          }
          onRetry={() => void usage.refetch()}
        />
      ) : usage.isPending ? (
        <div aria-label="Loading usage" className="space-y-3">
          <Skeleton className="h-28" />
          <Skeleton className="h-64" />
        </div>
      ) : (
        data && (
          <>
            <p className="text-[11px] text-muted-foreground">
              {new Date(data.window.from).toLocaleString()} –{" "}
              {new Date(data.window.to).toLocaleString()} ·{" "}
              {data.freshness && (
                <>
                  {new Date(data.freshness.rolled_up_through) <
                  new Date(data.window.from)
                    ? `Backfilling history · rollups complete through ${new Date(data.freshness.rolled_up_through).toLocaleString()}`
                    : `Live · includes ${data.freshness.tail_rows.toLocaleString()} unfolded rows`}
                  {data.freshness.validated === false && " · Updating totals"}{" "}
                  ·{" "}
                </>
              )}
              {formatNumber(data.totals.events)} metered events ·{" "}
              {formatNumber(data.totals.unique_services)} services
            </p>
            {data.totals.events === 0 ? (
              <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
                <ArticleIcon className="h-48 w-48 text-muted-foreground/30" />
                <p className="text-[12px] font-medium text-muted-foreground">
                  No usage in this window.
                </p>
                <p className="text-[12px] text-muted-foreground">
                  Try another time range or clear filters. New usage appears
                  after services process requests with usage metering enabled.
                </p>
              </div>
            ) : (
              <>
                {data.totals.unknown_cost_events > 0 && (
                  <p className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
                    Costs are partial:{" "}
                    {formatNumber(data.totals.unknown_cost_events)} historical
                    events have no cached rate.
                  </p>
                )}
                <p className="text-[11px] text-muted-foreground">
                  Gross costs use settled amounts or current rates for legacy
                  events. Wallet{" "}
                  {formatEstimatedCredits(data.totals.wallet_cost_micros)} ·
                  Grants {formatEstimatedCredits(data.totals.grant_cost_micros)}{" "}
                  · Allowances{" "}
                  {formatEstimatedCredits(data.totals.allowance_cost_micros)}.
                  Token total is input + output; provider cache counts can
                  overlap input. Quantities include each billing component and
                  resale event.
                </p>
                <section className="space-y-3">
                  <h2 className="text-[15px] font-semibold">By service</h2>
                  <ServiceTable services={data.by_service} />
                </section>
                <section className="space-y-3">
                  <h2 className="text-[15px] font-semibold">
                    Platform key vs own key
                  </h2>
                  <StatsTable
                    firstHeading="Credential type"
                    rows={data.by_credential_class.map((lane) => ({
                      key: lane.credential_class,
                      label: (
                        <Badge variant="secondary">
                          {credentialClassLabel(lane.credential_class)}
                        </Badge>
                      ),
                      usage: lane,
                    }))}
                  />
                </section>
                <section className="space-y-3">
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <div>
                      <h2 className="text-[15px] font-semibold">Top users</h2>
                      <p className="text-[11px] text-muted-foreground">
                        Ranked by user, service, and billing owner.
                      </p>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <Choice
                        label="Ranking sort"
                        value={search.sort}
                        onChange={(sort) =>
                          change({ sort: sort as AdminUsageSearch["sort"] })
                        }
                        options={USAGE_SORTS.map((value) => ({
                          value,
                          label: SORT_LABELS[value],
                        }))}
                      />
                      {search.sort === "quantity" && (
                        <Choice
                          label="Ranking metric"
                          value={search.metric}
                          onChange={(metric) =>
                            change({
                              metric: metric as AdminUsageSearch["metric"],
                            })
                          }
                          options={BILLING_METRICS.map((value) => ({
                            value,
                            label: metricLabel(value),
                          }))}
                        />
                      )}
                    </div>
                  </div>
                  <RankingTable rows={data.ranking} search={detailSearch} />
                  {data.ranking.length === 0 && (
                    <p className="text-[12px] text-muted-foreground">
                      No ranking rows on this page.
                    </p>
                  )}
                  <div className="flex flex-wrap items-center justify-between gap-3 text-[11px] text-text-tertiary">
                    <span>
                      {formatNumber(data.ranking_total)} user/service entries
                    </span>
                    <Choice
                      label="Rows per page"
                      value={String(search.per_page)}
                      onChange={(value) => change({ per_page: Number(value) })}
                      options={[25, 50, 100].map((value) => ({
                        value: String(value),
                        label: `${value} per page`,
                      }))}
                    />
                    <div className="flex items-center gap-2">
                      <Button
                        variant="outline"
                        size="icon"
                        aria-label="Previous page"
                        disabled={search.page <= 1 || usage.isFetching}
                        onClick={() => change({ page: search.page - 1 })}
                      >
                        <ChevronLeft className="size-3" />
                      </Button>
                      <span>
                        Page {search.page} of {pages}
                      </span>
                      <Button
                        variant="outline"
                        size="icon"
                        aria-label="Next page"
                        disabled={search.page >= pages || usage.isFetching}
                        onClick={() => change({ page: search.page + 1 })}
                      >
                        <ChevronRight className="size-3" />
                      </Button>
                    </div>
                  </div>
                </section>
              </>
            )}
          </>
        )
      )}
    </div>
  );
}
