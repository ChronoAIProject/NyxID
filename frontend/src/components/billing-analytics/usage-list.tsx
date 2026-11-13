import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import {
  Fragment,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { ChevronDown, ChevronLeft, ChevronRight } from "lucide-react";
import type { AnalyticsFilters } from "@/schemas/usage-analytics";
import {
  useAdminUsage,
  usePreloadAdminUsageDetails,
  type AdminUsageParams,
} from "@/hooks/use-admin-usage";
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
import { MEASURE_LABELS } from "@/lib/usage-analytics";
import { TokenMetricPicker, TokenOverlapHint } from "./token-metric-picker";
import {
  TOKEN_METRICS,
  sumTokenMetrics,
  type TokenMetric,
} from "./token-metrics";
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
  get requests() { return translateUi("Requests"); },
  get quantity() { return translateUi("Metric quantity"); },
  get cost() { return translateUi("Gross cost"); },
  get total_tokens() { return translateUi("Total tokens"); },
  get prompt_tokens() { return translateUi("Input tokens"); },
  get completion_tokens() { return translateUi("Output tokens"); },
  get cached_tokens() { return translateUi("Cache-read tokens"); },
  get cache_creation_tokens() { return translateUi("Cache-write tokens"); },
  get audio_input_tokens() { return translateUi("Voice input tokens"); },
  get audio_output_tokens() { return translateUi("Voice output tokens"); },
};
function listUsageParams(
  search: AdminUsageSearch,
  filters: AnalyticsFilters,
): AdminUsageParams {
  return {
    ...search,
    period: filters.period,
    from: filters.from ?? undefined,
    to: filters.to ?? undefined,
    user: undefined,
    service: undefined,
    services: filters.services,
    actors: filters.actors,
    owners: filters.owners,
  };
}

function detailUsageParams(
  search: AdminUsageParams,
  data: { window: { from: string; to: string } } | undefined,
): AdminUsageParams {
  return data
    ? {
        ...search,
        period: "custom",
        from: data.window.from,
        to: data.window.to,
        sort: "requests",
        metric: "tokens",
        page: 1,
        per_page: 25,
      }
    : search;
}

export function AdminUsagePreload({
  filters,
  search,
}: {
  filters: AnalyticsFilters;
  search: AdminUsageSearch;
}) {
  const params = useMemo(
    () => listUsageParams(search, filters),
    [search, filters],
  );
  const usage = useAdminUsage(params);
  const detail = useMemo(
    () => detailUsageParams(params, usage.data),
    [params, usage.data],
  );
  usePreloadAdminUsageDetails(usage.data, detail);
  return null;
}

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
  useUiTranslation();
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
  useUiTranslation();
  return (
    <div className="min-w-0">
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="type-row-title">{user.display_name}</span>
        </TooltipTrigger>
        <TooltipContent>{user.id}</TooltipContent>
      </Tooltip>
      {user.email && (
        <div className="break-all text-13 text-muted-foreground">
          {user.email}
        </div>
      )}
      {user.user_type === "org" && (
        <Badge variant="secondary">{translateUi("Organization")}</Badge>
      )}
    </div>
  );
}
function Quantities({
  usage,
  tokenView,
}: {
  usage: AdminUsageStats;
  tokenView: readonly TokenMetric[];
}) {
  useUiTranslation();
  const allSelected = tokenView.length === TOKEN_METRICS.length;
  return (
    <div className="space-y-1 font-mono text-13 tabular-nums">
      {Object.entries(usage.quantities).map(([metric, quantity]) => (
        <div key={metric}>
          {formatNumber(quantity)} {metricLabel(metric)}
        </div>
      ))}
      {(usage.total_tokens > 0 ||
        usage.cached_tokens > 0 ||
        usage.cache_creation_tokens > 0) && (
        <div className="text-muted-foreground">
          {allSelected ? (
            <>
              {formatNumber(usage.total_tokens)}{translateUi(" total tokens · in")}{" "}
              {formatNumber(usage.prompt_tokens)}{translateUi(" · out")}{" "}
              {formatNumber(usage.completion_tokens)}
              <br />{translateUi("cache read ")}{formatNumber(usage.cached_tokens)}{translateUi(" · cache write")}{" "}
              {formatNumber(usage.cache_creation_tokens)}
              {usage.audio_input_tokens + usage.audio_output_tokens > 0 && (
                <>
                  <br />{translateUi("voice in ")}{formatNumber(usage.audio_input_tokens)}{translateUi(" · voice out")}{" "}
                  {formatNumber(usage.audio_output_tokens)}
                </>
              )}
            </>
          ) : tokenView.length === 1 ? (
            <>
              {formatNumber(usage[tokenView[0]!])}{" "}
              {MEASURE_LABELS[tokenView[0]!].toLowerCase()}
            </>
          ) : (
            <>
              <span className="inline-flex items-center gap-1">
                {formatNumber(sumTokenMetrics(usage, tokenView))}{translateUi(" selected tokens")}<TokenOverlapHint selected={tokenView} />
              </span>
              {tokenView.map((metric) => (
                <div key={metric}>
                  {formatNumber(usage[metric])}{" "}
                  {MEASURE_LABELS[metric].toLowerCase()}
                </div>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}
function Cost({ usage }: { usage: AdminUsageStats }) {
  useUiTranslation();
  return (
    <div className="space-y-1 font-mono text-13 tabular-nums">
      <span>
        {formatEstimatedCredits(usage.gross_cost ?? usage.gross_cost_micros)}
      </span>
      <div className="text-muted-foreground">{translateUi("Wallet")}{" "}
        {formatEstimatedCredits(usage.wallet_cost ?? usage.wallet_cost_micros)}
        <br />{translateUi("Grants")}{" "}
        {formatEstimatedCredits(usage.grant_cost ?? usage.grant_cost_micros)}
        <br />{translateUi("Allowance")}{" "}
        {formatEstimatedCredits(
          usage.allowance_cost ?? usage.allowance_cost_micros,
        )}
      </div>
      {usage.unknown_cost_events > 0 && (
        <Badge variant="warning">{translateUi("Partial estimate")}</Badge>
      )}
    </div>
  );
}
function ServiceName({
  service,
}: {
  service: Pick<AdminUsageService, "service_name" | "service_slug">;
}) {
  useUiTranslation();
  return (
    <div className="type-row-title">
      {service.service_name}
      {service.service_slug && (
        <div className="type-secondary text-muted-foreground">
          {service.service_slug}
        </div>
      )}
    </div>
  );
}
function StatsTable({
  rows,
  firstHeading = "Service",
  tokenView,
}: {
  rows: { key: string; label: ReactNode; usage: AdminUsageStats }[];
  firstHeading?: string;
  tokenView: readonly TokenMetric[];
}) {
  useUiTranslation();
  return (
    <>
      <div className="hidden overflow-hidden rounded-lg border border-border bg-card md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{firstHeading}</TableHead>
              <TableHead>{translateUi("Requests")}</TableHead>
              <TableHead>{translateUi("Usage")}</TableHead>
              <TableHead>{translateUi("Gross cost")}</TableHead>
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
                  <Quantities usage={row.usage} tokenView={tokenView} />
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
            <div className="font-mono text-13">
              {formatNumber(row.usage.requests)}{translateUi(" requests")}</div>
            <Quantities usage={row.usage} tokenView={tokenView} />
            <Cost usage={row.usage} />
          </div>
        ))}
      </div>
    </>
  );
}
function ServiceTable({
  services,
  tokenView,
}: {
  services: AdminUsageService[];
  tokenView: readonly TokenMetric[];
}) {
  useUiTranslation();
  return (
    <StatsTable
      tokenView={tokenView}
      rows={services.map((service) => ({
        key: `${service.service_id}:${service.service_slug}`,
        label: (
          <div className="space-y-2">
            <ServiceName service={service} />
            <span className="text-13 text-muted-foreground">
              {formatNumber(service.unique_users)}{translateUi(" users")}</span>
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
            {service.by_operation.length > 0 && (
              <DataTableBadgeCell>
                {service.by_operation.map((operation) => (
                  <Badge key={operation.operation} variant="info">
                    {operation.operation}:{" "}
                    <span className="font-mono">
                      {formatNumber(operation.requests)}
                    </span>
                  </Badge>
                ))}
              </DataTableBadgeCell>
            )}
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
  tokenView,
}: {
  user: AdminUsageIdentity;
  search: AdminUsageParams;
  tokenView: readonly TokenMetric[];
}) {
  useUiTranslation();
  const usage = useAdminUsage({
    ...search,
    user: user.id,
    service: undefined,
    page: 1,
  });
  return (
    <div className="space-y-3 p-3">
      <p className="type-row-title">{translateUi("Services for ")}{user.display_name}</p>
      {usage.isPending ? (
        <Skeleton className="h-20" />
      ) : usage.isError ? (
        <ErrorBanner
          message={translateUi("Could not load user breakdown.")}
          onRetry={() => void usage.refetch()}
        />
      ) : (
        usage.data && (
          <ServiceTable
            services={usage.data.by_service}
            tokenView={tokenView}
          />
        )
      )}
    </div>
  );
}
function RankingTable({
  rows,
  search,
  tokenView,
}: {
  rows: AdminUsageRanking[];
  search: AdminUsageParams;
  tokenView: readonly TokenMetric[];
}) {
  useUiTranslation();
  const [expanded, setExpanded] = useState<string | null>(null);
  const rowKey = (row: AdminUsageRanking) =>
    `${row.user.id}:${row.billing_owner?.id}:${row.service_id}:${row.service_slug}`;
  const person = (row: AdminUsageRanking) => (
    <div className="space-y-2">
      <Identity user={row.user} />
      {row.billing_owner && (
        <div className="border-l border-border pl-2">
          <span className="text-12 text-muted-foreground">{translateUi("Billed to")}</span>
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
        >{translateUi("Services ")}<ChevronDown className="size-3" />
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
              <TableHead>{translateUi("User")}</TableHead>
              <TableHead>{translateUi("Service")}</TableHead>
              <TableHead>{translateUi("Usage")}</TableHead>
              <TableHead>{translateUi("Requests")}</TableHead>
              <TableHead>{translateUi("Gross cost")}</TableHead>
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
                    <Quantities usage={row} tokenView={tokenView} />
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
                      <UserServices
                        user={row.user}
                        search={search}
                        tokenView={tokenView}
                      />
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
            <p className="font-mono text-13">
              {formatNumber(row.requests)}{translateUi(" requests")}</p>
            <Quantities usage={row} tokenView={tokenView} />
            <Cost usage={row} />
            {expanded === rowKey(row) && (
              <UserServices
                user={row.user}
                search={search}
                tokenView={tokenView}
              />
            )}
          </div>
        ))}
      </div>
    </>
  );
}
export function AdminUsageList({ filters }: { filters: AnalyticsFilters }) {
  useUiTranslation();
  const routeSearch = useSearch({ from: "/dashboard/admin/usage" });
  const search = normalizeAdminUsageSearch(routeSearch);
  const navigate = useNavigate();
  const [tokenView, setTokenView] = useState<TokenMetric[]>([...TOKEN_METRICS]);
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
  const effectiveSearch = listUsageParams(search, filters);
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
  const detailSearch = detailUsageParams(effectiveSearch, data);
  return (
    <div className="space-y-6">
      {rangeError ? (
        <p role="alert" className="text-14 text-destructive">
          {translateUi(rangeError)}
        </p>
      ) : usage.isError ? (
        <ErrorBanner
          message={
            usage.error instanceof Error
              ? usage.error.message
              : translateUi("Failed to load usage.")
          }
          onRetry={() => void usage.refetch()}
        />
      ) : usage.isPending ? (
        <div aria-label={translateUi("Loading usage")} className="space-y-3">
          <Skeleton className="h-28" />
          <Skeleton className="h-64" />
        </div>
      ) : (
        data && (
          <>
            <p className="text-13 text-muted-foreground">
              {new Date(data.window.from).toLocaleString(getFormattingLocale())} –{" "}
              {new Date(data.window.to).toLocaleString(getFormattingLocale())} ·{" "}
              {data.freshness && (
                <>
                  {new Date(data.freshness.rolled_up_through) <
                  new Date(data.window.from)
                    ? translateUi("Backfilling history · rollups complete through {{value1}}", { value1: new Date(data.freshness.rolled_up_through).toLocaleString(getFormattingLocale()) })
                    : translateUi("Live · includes {{value1}} unfolded rows", { value1: data.freshness.tail_rows.toLocaleString(getFormattingLocale()) })}
                  {data.freshness.validated === false && " · Updating totals"}{" "}
                  ·{" "}
                </>
              )}
              {formatNumber(data.totals.events)}{translateUi(" metered events ·")}{" "}
              {formatNumber(data.totals.unique_services)}{translateUi(" services")}</p>
            {data.totals.events === 0 ? (
              <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
                <ArticleIcon className="h-48 w-48 text-muted-foreground/30" />
                <p className="type-row-title text-muted-foreground">{translateUi("No usage in this window.")}</p>
                <p className="text-14 text-muted-foreground">{translateUi("Try another time range or clear filters. New usage appears after services process requests with usage metering enabled.")}</p>
              </div>
            ) : (
              <>
                {data.totals.unknown_cost_events > 0 && (
                  <p className="rounded-lg border border-dashed border-border px-4 py-3 text-14 text-muted-foreground">{translateUi("Costs are partial:")}{" "}
                    {formatNumber(data.totals.unknown_cost_events)}{translateUi(" historical events have no cached rate.")}</p>
                )}
                <p className="text-13 text-muted-foreground">{translateUi("Gross costs use settled amounts or current rates for legacy events. Wallet")}{" "}
                  {formatEstimatedCredits(
                    data.totals.wallet_cost ?? data.totals.wallet_cost_micros,
                  )}{" "}{translateUi("· Grants")}{" "}
                  {formatEstimatedCredits(
                    data.totals.grant_cost ?? data.totals.grant_cost_micros,
                  )}{" "}{translateUi("· Allowances")}{" "}
                  {formatEstimatedCredits(
                    data.totals.allowance_cost ??
                      data.totals.allowance_cost_micros,
                  )}
                  {translateUi(". Token total is input + output; provider cache counts can overlap input. Quantities include each billing component and resale event. ")}</p>
                <div className="flex justify-end">
                  <TokenMetricPicker
                    label={translateUi("Token view")}
                    selected={tokenView}
                    onChange={setTokenView}
                    allLabel={translateUi("All token types")}
                  />
                </div>
                <section className="space-y-3">
                  <h2 className="type-card-title">{translateUi("By service")}</h2>
                  <ServiceTable
                    services={data.by_service}
                    tokenView={tokenView}
                  />
                </section>
                <section className="space-y-3">
                  <h2 className="type-card-title">{translateUi("Platform key vs own key")}</h2>
                  <StatsTable
                    tokenView={tokenView}
                    firstHeading={translateUi("Credential type")}
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
                      <h2 className="type-card-title">{translateUi("Top users")}</h2>
                      <p className="text-13 text-muted-foreground">{translateUi("Ranked by user, service, and billing owner.")}</p>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <Choice
                        label={translateUi("Ranking sort")}
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
                          label={translateUi("Ranking metric")}
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
                  <RankingTable
                    rows={data.ranking}
                    search={detailSearch}
                    tokenView={tokenView}
                  />
                  {data.ranking.length === 0 && (
                    <p className="text-14 text-muted-foreground">{translateUi("No ranking rows on this page.")}</p>
                  )}
                  <div className="flex flex-wrap items-center justify-between gap-3 text-13 text-text-tertiary">
                    <span>
                      {formatNumber(data.ranking_total)}{translateUi(" user/service entries")}</span>
                    <Choice
                      label={translateUi("Rows per page")}
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
                        aria-label={translateUi("Previous page")}
                        disabled={search.page <= 1 || usage.isFetching}
                        onClick={() => change({ page: search.page - 1 })}
                      >
                        <ChevronLeft className="size-3" />
                      </Button>
                      <span>{translateUi("Page ")}{search.page}{translateUi(" of ")}{pages}
                      </span>
                      <Button
                        variant="outline"
                        size="icon"
                        aria-label={translateUi("Next page")}
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
