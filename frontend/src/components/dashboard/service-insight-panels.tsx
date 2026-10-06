import { useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  Activity,
  ArrowUpRight,
  Bot,
  CreditCard,
  UsersRound,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { metricLabel } from "@/schemas/billing-metrics";
import type { BillingUsagePeriod } from "@/schemas/billing";
import { useBillingUsage } from "@/hooks/use-billing";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Line,
  LineChart,
  XAxis,
  YAxis,
} from "recharts";
import {
  ChartContainer,
  ChartTooltip,
  type ChartConfig,
} from "@/components/ui/chart";
import {
  serviceUsageDaily,
  serviceUsageSummary,
  serviceUsageTrend,
  type ServiceUsageDay,
  type ServiceUsageSummary,
  type ServiceUsageWindow,
} from "@/lib/service-usage";
import { formatExactCredits, hasCredits, parseCredits } from "@/lib/credits";
import { plainBilling } from "@/lib/billing-plain";
import type {
  ConfiguredCatalogEntry,
  ServiceInsight,
} from "@/schemas/service-insights";
import {
  useServiceInsights,
  type ServiceInsightsState,
} from "@/hooks/use-service-insights";
import {
  accessReasonLabel,
  callerKindLabel,
  callerLabel,
  outcomeLabel,
  recordedSourceLabel,
} from "@/lib/service-insights";
import { cn, formatDateTime } from "@/lib/utils";
import type { KeyInfo } from "@/types/keys";

export type InsightPanel = "access" | "requests" | "billing";

export function InsightsUnavailable({
  state,
}: {
  readonly state: ServiceInsightsState;
}) {
  return (
    <div
      className="flex items-center justify-between gap-4 p-3 text-xs text-muted-foreground"
      role="status"
    >
      <p>
        {state.status === "loading"
          ? "Loading billing and caller information…"
          : state.status === "restricted"
            ? "You do not have permission to view these connection insights."
            : state.status === "unavailable"
              ? "This server does not provide connection billing and caller insights yet."
              : "Connection insights could not be loaded."}
      </p>
      {state.status !== "loading" && (
        <Button size="sm" variant="outline" onClick={state.refresh}>
          Retry
        </Button>
      )}
    </div>
  );
}

const USAGE_PERIODS: readonly [BillingUsagePeriod, string][] = [
  ["24h", "Last 24 hours"],
  ["7d", "Last 7 days"],
  ["30d", "Last 30 days"],
  ["90d", "Last 90 days"],
];

const usageChartConfig = {
  value: { label: "Usage", color: "var(--color-primary)" },
} satisfies ChartConfig;

/** What the trend line plots: calls, one recorded metric, or credits. */
type TrendMeasure = "calls" | "credits" | `metric:${string}`;

function trendValue(day: ServiceUsageDay, measure: TrendMeasure) {
  if (measure === "calls") return day.calls;
  // Unsettled days have no exact charge yet; leave a gap, not a fake zero.
  if (measure === "credits")
    return day.charged == null ? null : Number(day.charged);
  const metric = measure.slice("metric:".length);
  return day.quantities.find((q) => q.metric === metric)?.quantity ?? 0;
}

function trendLabel(measure: TrendMeasure) {
  if (measure === "calls") return "Calls";
  if (measure === "credits") return "Credits";
  const label = metricLabel(measure.slice("metric:".length));
  return label.charAt(0).toUpperCase() + label.slice(1);
}

const shortDay = (day: string) =>
  new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });

/** Where charged credits came from, in the order NyxID spends them. */
function fundingParts(source: {
  readonly allowance: string;
  readonly grant: string;
  readonly wallet: string;
}) {
  return (
    [
      ["Free allowance", source.allowance],
      ["Free credit grants", source.grant],
      ["Wallet", source.wallet],
    ] as const
  ).filter(([, value]) => hasCredits(value));
}

function UsageDayTooltip({
  active,
  payload,
}: {
  readonly active?: boolean;
  readonly payload?: readonly { payload: { source: ServiceUsageDay } }[];
}) {
  const day = active ? payload?.[0]?.payload.source : undefined;
  if (!day) return null;
  return (
    <div className="rounded-lg border border-border/60 bg-popover px-2.5 py-1.5 text-xs shadow-md">
      <p className="font-medium">{shortDay(day.day)} (UTC)</p>
      <p className="tabular-nums">
        {day.calls.toLocaleString()} {day.calls === 1 ? "call" : "calls"}
      </p>
      {day.quantities.map(({ metric, quantity }) => (
        <p key={metric} className="text-muted-foreground tabular-nums">
          {quantity.toLocaleString()} {metricLabel(metric, quantity)}
        </p>
      ))}
      {day.charged == null ? (
        <p className="text-muted-foreground">Credits still being calculated</p>
      ) : (
        hasCredits(day.charged) && (
          <p className="text-muted-foreground tabular-nums">
            {formatExactCredits(day.charged)} credits
          </p>
        )
      )}
      {fundingParts(day).length > 0 && (
        <p className="text-muted-foreground tabular-nums">
          Paid from{" "}
          {fundingParts(day)
            .map(
              ([label, value]) =>
                `${label.toLowerCase()} ${formatExactCredits(value)}`,
            )
            .join(" · ")}
        </p>
      )}
    </div>
  );
}

function UsageTrendChart({
  series,
  measures,
}: {
  readonly series: ServiceUsageDay[];
  readonly measures: readonly TrendMeasure[];
}) {
  const [picked, setPicked] = useState<TrendMeasure>("calls");
  const measure = measures.includes(picked) ? picked : "calls";
  const data = series.map((day) => ({
    day: day.day,
    value: trendValue(day, measure),
    source: day,
  }));
  const label = trendLabel(measure);
  return (
    <figure aria-label={`${label} per day`} className="space-y-2">
      {measures.length > 1 && (
        <div
          role="group"
          aria-label="Chart shows"
          className="inline-flex rounded-md border border-border/60 p-0.5"
        >
          {measures.map((option) => (
            <button
              key={option}
              type="button"
              aria-pressed={option === measure}
              onClick={() => setPicked(option)}
              className={cn(
                "rounded px-2 py-0.5 text-[11px] text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring",
                option === measure && "bg-muted font-medium text-foreground",
              )}
            >
              {trendLabel(option)}
            </button>
          ))}
        </div>
      )}
      <ChartContainer
        config={usageChartConfig}
        className="aspect-auto h-32 w-full"
      >
        <LineChart
          accessibilityLayer
          data={data}
          margin={{ top: 6, right: 6, bottom: 0, left: 0 }}
        >
          <CartesianGrid vertical={false} strokeOpacity={0.4} />
          <XAxis
            dataKey="day"
            tickLine={false}
            axisLine={false}
            tickMargin={6}
            minTickGap={24}
            tickFormatter={shortDay}
          />
          <YAxis
            width={44}
            tickLine={false}
            axisLine={false}
            allowDecimals={measure === "credits"}
            tickCount={3}
            tickFormatter={(value: number) =>
              value.toLocaleString(undefined, {
                notation: "compact",
                maximumSignificantDigits: 3,
              })
            }
          />
          <ChartTooltip
            cursor={{ stroke: "var(--color-border)" }}
            content={<UsageDayTooltip />}
          />
          <Line
            type="monotone"
            dataKey="value"
            name={label}
            stroke="var(--color-value)"
            strokeWidth={2}
            dot={series.length <= 7}
            activeDot={{ r: 4 }}
            isAnimationActive={false}
          />
        </LineChart>
      </ChartContainer>
      <figcaption className="sr-only">
        {data
          .filter((point) => point.value)
          .map((point) => `${shortDay(point.day)}: ${point.value} ${label}`)
          .join(", ") || `No ${label.toLowerCase()} in this period`}
      </figcaption>
    </figure>
  );
}

const FUNDING_COLORS: Record<string, string> = {
  "Free allowance": "var(--color-success)",
  "Free credit grants": "var(--chart-1)",
  Wallet: "var(--color-primary)",
};

/** One bar split by where the period's credits came from. */
function FundingBar({ summary }: { readonly summary: ServiceUsageSummary }) {
  const parts = fundingParts(summary).map(([label, value]) => ({
    label,
    value,
    pico: parseCredits(value),
  }));
  const total = parts.reduce((sum, part) => sum + part.pico, 0n);
  if (total === 0n) return null;
  return (
    <div className="space-y-1.5">
      <p className="text-muted-foreground">Where the credits came from</p>
      <div
        className="flex h-2 overflow-hidden rounded-full bg-muted"
        aria-hidden="true"
      >
        {parts.map((part) => (
          <div
            key={part.label}
            style={{
              width: `${Number((part.pico * 10_000n) / total) / 100}%`,
              background: FUNDING_COLORS[part.label],
            }}
          />
        ))}
      </div>
      <ul className="flex flex-wrap gap-x-4 gap-y-1 text-[11px]">
        {parts.map((part) => (
          <li key={part.label} className="inline-flex items-center gap-1.5">
            <span
              className="size-2 rounded-full"
              style={{ background: FUNDING_COLORS[part.label] }}
              aria-hidden="true"
            />
            {part.label}{" "}
            <span className="tabular-nums text-muted-foreground">
              {formatExactCredits(part.value)} (
              {Math.round(Number((part.pico * 1000n) / total) / 10)}%)
            </span>
          </li>
        ))}
        {!hasCredits(summary.wallet) && (
          <li className="text-muted-foreground">Nothing from your wallet</li>
        )}
      </ul>
    </div>
  );
}

/** Ranked horizontal bars, capped at five rows. */
function BreakdownBars({
  title,
  items,
}: {
  readonly title: string;
  readonly items: readonly { label: string; value: number }[];
}) {
  const max = Math.max(1, ...items.map((item) => item.value));
  const shown = items.slice(0, 5);
  return (
    <div className="space-y-1.5">
      <p className="text-muted-foreground">{title}</p>
      <ul className="space-y-1">
        {shown.map((item) => (
          <li
            key={item.label}
            className="grid grid-cols-[minmax(0,12rem)_1fr_3rem] items-center gap-2"
          >
            <span className="truncate" title={item.label}>
              {item.label}
            </span>
            <span className="h-2 rounded-full bg-muted" aria-hidden="true">
              <span
                className="block h-full rounded-full bg-primary"
                style={{ width: `${(item.value / max) * 100}%` }}
              />
            </span>
            <span className="text-right tabular-nums">
              {item.value.toLocaleString()}
            </span>
          </li>
        ))}
      </ul>
      {items.length > shown.length && (
        <p className="text-[11px] text-muted-foreground">
          +{items.length - shown.length} more
        </p>
      )}
    </div>
  );
}

const windowChartConfig = {
  perDay: { label: "Calls per day", color: "var(--color-primary)" },
} satisfies ChartConfig;

/**
 * Average calls per day over recent windows, built from period totals so it
 * works before the server reports usage by day.
 */
function UsageWindows({ slug }: { readonly slug: string }) {
  const day = useBillingUsage("24h");
  const week = useBillingUsage("7d");
  const month = useBillingUsage("30d");
  const quarter = useBillingUsage("90d");
  const queries = [day, week, month, quarter];
  if (queries.some((query) => query.isPending))
    return <p className="text-muted-foreground">Loading recent trend…</p>;
  if (!day.data || !week.data || !month.data || !quarter.data) return null;
  const windows = serviceUsageTrend(
    {
      "24h": day.data.rows,
      "7d": week.data.rows,
      "30d": month.data.rows,
      "90d": quarter.data.rows,
    },
    slug,
  );
  return (
    <figure aria-label="Average calls per day" className="space-y-1.5">
      <p className="text-muted-foreground">
        Recent trend · average calls per day
      </p>
      <ChartContainer
        config={windowChartConfig}
        className="aspect-auto h-28 w-full"
      >
        <BarChart
          accessibilityLayer
          data={windows}
          margin={{ top: 4, right: 0, bottom: 0, left: 0 }}
        >
          <CartesianGrid vertical={false} strokeOpacity={0.4} />
          <XAxis
            dataKey="label"
            tickLine={false}
            axisLine={false}
            tickMargin={6}
          />
          <YAxis
            width={36}
            tickLine={false}
            axisLine={false}
            tickCount={3}
            tickFormatter={(value: number) =>
              value.toLocaleString(undefined, { maximumFractionDigits: 1 })
            }
          />
          <ChartTooltip
            cursor={{ fill: "var(--color-muted)", opacity: 0.4 }}
            content={<WindowTooltip />}
          />
          <Bar
            dataKey="perDay"
            fill="var(--color-perDay)"
            radius={[4, 4, 0, 0]}
            maxBarSize={48}
            isAnimationActive={false}
          />
        </BarChart>
      </ChartContainer>
      <figcaption className="text-[11px] text-muted-foreground">
        {windows
          .map(
            (slot) =>
              `${slot.label}: ${slot.calls.toLocaleString()} ${slot.calls === 1 ? "call" : "calls"}`,
          )
          .join(" · ")}
        . A day-by-day graph replaces this once NyxID reports usage by day.
      </figcaption>
    </figure>
  );
}

function WindowTooltip({
  active,
  payload,
}: {
  readonly active?: boolean;
  readonly payload?: readonly { payload: ServiceUsageWindow }[];
}) {
  const slot = active ? payload?.[0]?.payload : undefined;
  if (!slot) return null;
  return (
    <div className="rounded-lg border border-border/60 bg-popover px-2.5 py-1.5 text-xs shadow-md">
      <p className="font-medium">{slot.label}</p>
      <p className="tabular-nums">
        {slot.calls.toLocaleString()} {slot.calls === 1 ? "call" : "calls"}
      </p>
      <p className="text-muted-foreground tabular-nums">
        {slot.perDay.toLocaleString(undefined, { maximumFractionDigits: 1 })}{" "}
        per day on average
      </p>
    </div>
  );
}

function ConnectionUsage({
  connection,
  freeNow,
}: {
  readonly connection: KeyInfo;
  /** The connection is free on NyxID today, so charged usage needs a reason. */
  readonly freeNow: boolean;
}) {
  const [period, setPeriod] = useState<BillingUsagePeriod>("30d");
  // One daily request feeds both the totals and the chart.
  const usage = useBillingUsage(period, period === "24h" ? undefined : "day");
  const summary = usage.data
    ? serviceUsageSummary(usage.data.rows, connection.slug)
    : null;
  const daily = usage.data
    ? serviceUsageDaily(usage.data.rows, connection.slug, period)
    : null;
  const org =
    connection.credential_source?.type === "org"
      ? connection.credential_source.org_name
      : null;
  return (
    <section
      className="space-y-3 border-t border-border/60 pt-4 text-xs"
      aria-labelledby={`usage-history-${connection.id}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h5
          id={`usage-history-${connection.id}`}
          className="text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary"
        >
          Usage history
        </h5>
        <Select
          value={period}
          onValueChange={(value) => setPeriod(value as BillingUsagePeriod)}
        >
          <SelectTrigger aria-label="Usage period" className="h-8 w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {USAGE_PERIODS.map(([value, label]) => (
              <SelectItem key={value} value={value}>
                {label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {usage.isPending ? (
        <p className="text-muted-foreground">Loading usage…</p>
      ) : usage.isError ? (
        <p className="text-muted-foreground">Usage couldn’t load.</p>
      ) : !summary ? (
        <p className="text-muted-foreground">
          No usage recorded through <code>{connection.slug}</code> in this
          period.
        </p>
      ) : (
        <>
          <dl className="grid gap-3 sm:grid-cols-3">
            <div>
              <dt className="text-muted-foreground">Calls</dt>
              <dd className="mt-1 text-sm font-medium tabular-nums">
                {summary.calls.toLocaleString()}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Usage</dt>
              <dd className="mt-1 text-sm font-medium tabular-nums">
                {summary.quantities
                  .map(
                    ({ metric, quantity }) =>
                      `${quantity.toLocaleString()} ${metricLabel(metric, quantity)}`,
                  )
                  .join(" · ")}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">NyxID credits used</dt>
              <dd className="mt-1 text-sm font-medium tabular-nums">
                {!summary.billable
                  ? "None"
                  : summary.charged == null
                    ? "Still being calculated"
                    : formatExactCredits(summary.charged)}
              </dd>
            </div>
          </dl>
          {summary.billable && fundingParts(summary).length > 0 && (
            <FundingBar summary={summary} />
          )}
          {daily ? (
            <UsageTrendChart
              series={daily}
              measures={[
                "calls",
                // Requests repeat the call count, so they get no line of their own.
                ...summary.quantities
                  .filter(({ metric }) => metric !== "requests")
                  .map(({ metric }) => `metric:${metric}` as const),
                ...(summary.billable ? (["credits"] as const) : []),
              ]}
            />
          ) : (
            <UsageWindows slug={connection.slug} />
          )}
          {freeNow &&
            summary.charged != null &&
            hasCredits(summary.charged) && (
              <p className="text-amber-600 dark:text-amber-400">
                Credits were charged in this period even though this connection
                is free on NyxID now. They may come from an earlier price, or
                from another connection that shares{" "}
                <code>{connection.slug}</code>.
              </p>
            )}
          {(summary.agents.length > 1 || !!summary.agents[0]?.name) && (
            <BreakdownBars
              title="Who made the calls"
              items={summary.agents.map(({ name, calls }) => ({
                label: name ?? "You, signed in",
                value: calls,
              }))}
            />
          )}
          {summary.models.length > 1 ? (
            <BreakdownBars
              title="Calls by model"
              items={summary.models.map(({ name, calls }) => ({
                label: name,
                value: calls,
              }))}
            />
          ) : (
            summary.models[0] && (
              <p className="text-muted-foreground">
                Model:{" "}
                <span className="text-foreground">
                  {summary.models[0].name}
                </span>
              </p>
            )
          )}
        </>
      )}
      <p className="text-[11px] text-muted-foreground">
        Counts calls made through <code>{connection.slug}</code> that were
        billed to your personal account. Other connections using the same
        address are counted together.
        {org && ` Usage billed to ${org} isn’t included.`}
      </p>
    </section>
  );
}

function ConnectionBillingPanel({
  connection,
  insight,
  state,
  catalog,
}: {
  readonly connection: KeyInfo;
  readonly insight: ServiceInsight;
  readonly state: ServiceInsightsState;
  readonly catalog?: ConfiguredCatalogEntry;
}) {
  const [caller, setCaller] = useState("you");
  const selectedState = useServiceInsights(
    [connection],
    caller === "you" ? state : undefined,
    caller === "you" ? undefined : caller,
  );
  const bill =
    caller === "you"
      ? insight.billing
      : selectedState.connections.get(connection.id)?.billing;
  const plain = bill ? plainBilling(connection, bill, catalog) : null;
  // Settings say what should be charged; recorded usage says what was.
  const recent = useBillingUsage("7d");
  const recentCharged = recent.data
    ? serviceUsageSummary(recent.data.rows, connection.slug)?.charged
    : null;
  const chargedAnyway =
    plain?.verdict === "free" &&
    recentCharged != null &&
    hasCredits(recentCharged);
  return (
    <section
      className="space-y-4 p-3"
      aria-label={`Billing for ${connection.label}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="inline-flex items-center gap-2 text-sm font-medium">
          <CreditCard className="size-4 text-primary" /> Billing
        </h4>
        {insight.billing?.context !== "configuration" &&
          !!insight.usage?.access.keys.length && (
            <div className="flex items-center gap-2 text-xs">
              <span className="text-muted-foreground">Show for</span>
              <Select value={caller} onValueChange={setCaller}>
                <SelectTrigger
                  aria-label="Preview billing for"
                  className="h-8 w-52"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="you">You</SelectItem>
                  {insight.usage?.access.keys.map((key) => (
                    <SelectItem key={key.id} value={key.id}>
                      Agent key: {key.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
      </div>
      {plain ? (
        <>
          <div className="rounded-lg border border-border/60 bg-muted/20 p-3">
            <p className="text-sm font-medium">
              {chargedAnyway ? "Credits were charged recently" : plain.headline}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              {chargedAnyway
                ? `${formatExactCredits(recentCharged!)} NyxID credits were charged in the last 7 days for calls through ${connection.slug}, so don't treat this connection as free. ${plain.detail}`
                : plain.detail}
            </p>
          </div>
          <section
            className="space-y-3 border-t border-border/60 pt-4"
            aria-labelledby={`billing-details-${connection.id}`}
          >
            <h5
              id={`billing-details-${connection.id}`}
              className="text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary"
            >
              Details
            </h5>
            <dl className="grid gap-3 text-xs sm:grid-cols-3">
              <div>
                <dt className="text-muted-foreground">Whose key or app</dt>
                <dd className="mt-1 font-medium">{plain.key.title}</dd>
                {plain.key.note && (
                  <dd className="mt-0.5 text-[11px] text-muted-foreground">
                    {plain.key.note}
                  </dd>
                )}
              </div>
              <div>
                <dt className="text-muted-foreground">Who pays NyxID</dt>
                <dd className="mt-1 font-medium">{plain.payer}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">NyxID price</dt>
                <dd className="mt-1 font-medium">{plain.price}</dd>
              </div>
            </dl>
            {!!plain.tips.length && (
              <ul className="list-disc space-y-1 pl-4 text-[11px] text-muted-foreground">
                {plain.tips.map((tip) => (
                  <li key={tip}>{tip}</li>
                ))}
              </ul>
            )}
          </section>
        </>
      ) : (
        <InsightsUnavailable state={selectedState} />
      )}
      <ConnectionUsage
        connection={connection}
        freeNow={plain?.verdict === "free"}
      />
    </section>
  );
}

export function ConnectionInsightPanel({
  connection,
  insight,
  view,
  state,
  catalog,
}: {
  readonly connection: KeyInfo;
  readonly insight?: ServiceInsight;
  readonly view: InsightPanel;
  readonly state: ServiceInsightsState;
  readonly catalog?: ConfiguredCatalogEntry;
}) {
  const [showAllKeys, setShowAllKeys] = useState(false);
  if (!insight || (view === "billing" ? !insight.billing : !insight.usage))
    return <InsightsUnavailable state={state} />;
  const usage = insight.usage;
  if (view === "billing")
    return (
      <ConnectionBillingPanel
        connection={connection}
        insight={insight}
        state={state}
        catalog={catalog}
      />
    );
  if (!usage) return <InsightsUnavailable state={state} />;
  if (view === "access")
    return (
      <section
        className="space-y-3 p-3"
        aria-label={`Agent key access for ${connection.label}`}
      >
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h4 className="inline-flex items-center gap-2 text-sm font-medium">
            <UsersRound className="size-4 text-primary" />{" "}
            {usage.access.basis === "configuration"
              ? "Agent keys in scope"
              : "Agent keys with access"}
          </h4>
          <span className="text-[11px] text-muted-foreground">
            {usage.access.visibility === "own_keys"
              ? "Your keys only"
              : "Keys you manage"}{" "}
            · current scope
          </span>
        </div>
        {usage.access.keys.length ? (
          <div className="overflow-x-auto">
            <table
              className="w-full text-left text-xs"
              aria-label="Agent keys with access"
            >
              <thead className="text-muted-foreground">
                <tr>
                  <th className="py-2 pr-3 font-medium">Agent key</th>
                  <th className="px-3 py-2 font-medium">Access through</th>
                  <th className="px-3 py-2 font-medium">Credential</th>
                </tr>
              </thead>
              <tbody>
                {(showAllKeys
                  ? usage.access.keys
                  : usage.access.keys.slice(0, 3)
                ).map((key) => (
                  <tr key={key.id} className="border-t border-border/40">
                    <td className="py-2.5 pr-3">
                      <Link
                        to="/keys/api-key/$keyId"
                        params={{ keyId: key.id }}
                        className="inline-flex items-center gap-1.5 font-medium text-primary hover:underline"
                      >
                        <Bot className="size-3.5" />
                        {key.name}
                        <ArrowUpRight className="size-3" />
                      </Link>
                      {key.platform && (
                        <span className="ml-2 text-muted-foreground">
                          {key.platform}
                        </span>
                      )}
                    </td>
                    <td className="px-3 py-2.5">
                      {accessReasonLabel(key.permission)}
                    </td>
                    <td className="px-3 py-2.5">
                      {key.credential_override === null ? (
                        "Override not reported"
                      ) : key.credential_override ? (
                        <Badge variant="secondary">Credential override</Badge>
                      ) : (
                        "Connection default"
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">
            {usage.access.visibility === "unavailable"
              ? "Agent key inventory could not be loaded."
              : usage.access.incomplete
                ? "No matching keys in the available inventory. Some key inventories could not be checked."
                : "No agent keys in your permitted inventory currently include this connection in their scope."}
          </p>
        )}
        {usage.access.keys.length > 3 && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setShowAllKeys(!showAllKeys)}
          >
            {showAllKeys
              ? "Show fewer keys"
              : `Show all ${usage.access.keys.length} keys`}
          </Button>
        )}
        {usage.access.truncated && (
          <p className="text-xs text-muted-foreground">
            Showing a limited set of keys. Open Agent keys to review the full
            inventory.
          </p>
        )}
        <p className="text-[11px] text-muted-foreground">
          {usage.access.basis === "configuration"
            ? "Configured scope; live permissions and credentials are checked at execution. "
            : ""}
          {usage.access.incomplete && usage.access.keys.length > 0
            ? "Some key inventories could not be checked. "
            : ""}
          Scope access does not prove a working connection or previous use. Open
          a key to manage its service scope or credential override.
        </p>
      </section>
    );
  if (usage.activity.tracking === "unavailable")
    return (
      <section
        className="space-y-2 p-3 text-xs"
        aria-label={`Recent requests for ${connection.label}`}
      >
        <h4 className="font-medium">Request attribution unavailable</h4>
        <p className="text-muted-foreground">
          This server does not yet report which agent key or application used
          this exact connection. Configured key access is shown separately; it
          is not evidence of use.
        </p>
      </section>
    );
  return (
    <section
      className="space-y-3 p-3"
      aria-label={`Recent requests for ${connection.label}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="inline-flex items-center gap-2 text-sm font-medium">
          <Activity className="size-4 text-primary" /> Recent requests
        </h4>
        <span className="text-[11px] text-muted-foreground">
          {usage.activity.visibility === "own_requests"
            ? "Your requests"
            : "Visible requests"}{" "}
          · {usage.activity.period_days} days
        </span>
      </div>
      {usage.activity.requests.length ? (
        <div className="overflow-x-auto">
          <table
            className="w-full text-left text-xs"
            aria-label="Recent connection requests"
          >
            <thead className="text-muted-foreground">
              <tr>
                <th className="py-2 pr-3 font-medium">Caller</th>
                <th className="px-3 py-2 font-medium">Type / application</th>
                <th className="px-3 py-2 font-medium">Recorded layer</th>
                <th className="px-3 py-2 font-medium">Time</th>
                <th className="px-3 py-2 font-medium">Outcome</th>
              </tr>
            </thead>
            <tbody>
              {usage.activity.requests.map((request) => (
                <tr key={request.id} className="border-t border-border/40">
                  <td className="py-2.5 pr-3 font-medium">
                    {callerLabel(request.caller)}
                  </td>
                  <td className="px-3 py-2.5">
                    {callerKindLabel(request.caller.kind)}
                    {request.caller.kind !== "session" && (
                      <span className="mt-0.5 block text-muted-foreground">
                        {request.caller.app_name ??
                          (request.caller.app_id
                            ? "Application name not recorded"
                            : "Application not recorded")}
                      </span>
                    )}
                  </td>
                  <td className="px-3 py-2.5">
                    {recordedSourceLabel(request, connection)}
                  </td>
                  <td className="px-3 py-2.5">
                    <time dateTime={request.occurred_at}>
                      {formatDateTime(request.occurred_at)}
                    </time>
                  </td>
                  <td className="px-3 py-2.5">
                    {outcomeLabel(request.outcome)}
                    {request.response_status != null && (
                      <span className="ml-1 text-muted-foreground">
                        · {request.response_status}
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          No requests with exact connection attribution were recorded in this
          period.
        </p>
      )}
      {usage.activity.request_count > 0 && (
        <p className="text-[11px] text-muted-foreground">
          {usage.activity.request_count.toLocaleString()} recorded
          {usage.activity.request_count === 1 ? " request" : " requests"}
          {usage.activity.truncated
            ? ` · showing the latest ${usage.activity.requests.length}`
            : " in this period"}
        </p>
      )}
      <p className="text-[11px] text-muted-foreground">
        {usage.activity.tracking === "partial"
          ? "Tracking is partial. Older requests and unsupported request paths may not identify this connection. "
          : ""}
        A shared agent key identifies the key, not every application using it. A
        received response does not confirm stream completion or a settled
        charge.
      </p>
    </section>
  );
}
