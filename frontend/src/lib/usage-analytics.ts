import { usageRangeError } from "@/schemas/admin-usage";
import type {
  AnalyticsFilters,
  AnalyticsLayout,
  AnalyticsPanel,
  AnalyticsResult,
  AnalyticsView,
} from "@/schemas/usage-analytics";

export const MEASURE_LABELS: Record<AnalyticsPanel["measure"], string> = {
  cost: "Gross cost",
  requests: "Requests",
  events: "Billing events",
  exact_cost_events: "Exact-cost events",
  legacy_cost_events: "Legacy billing events",
  unknown_cost_events: "Uncosted events",
  total_tokens: "Total tokens",
  prompt_tokens: "Input tokens",
  completion_tokens: "Output tokens",
  cached_tokens: "Cache-read tokens",
  cache_creation_tokens: "Cache-write tokens",
  quantity: "Billed units",
  wallet_cost: "Wallet cost",
  grant_cost: "Grant cost",
  allowance_cost: "Allowance cost",
};
export const INTERVAL_LABELS = {
  auto: "Auto",
  hour: "Hourly",
  day: "Daily",
  week: "Weekly",
  month: "Monthly",
};
export const MEASURE_DESCRIPTIONS: Record<AnalyticsPanel["measure"], string> = {
  cost: "Total usage cost before grants and allowances, in credits.",
  requests:
    "Metered platform requests, counted once across billing components. Unmetered activity is not included.",
  events: "Metering records. One request can produce several billing events.",
  exact_cost_events: "Billing events with recorded settlement costs.",
  legacy_cost_events:
    "Historical billing events with estimated or unavailable cost. These can overlap uncosted events.",
  unknown_cost_events:
    "Billing events whose cost could not be determined. These leave gaps in cost charts.",
  total_tokens:
    "Provider-reported input + output tokens. Cache counts can overlap input; they are not added again.",
  prompt_tokens:
    "Provider-reported input tokens. Some providers include cached input here.",
  completion_tokens: "Provider-reported output tokens.",
  cached_tokens:
    "Provider-reported cached input reads. This can overlap input tokens.",
  cache_creation_tokens:
    "Provider-reported cache writes. Provider accounting varies; do not add this to total tokens.",
  quantity:
    "Sum of metered units in the selected billing metric. These can differ from provider-reported tokens.",
  wallet_cost: "Usage cost funded by purchased wallet credits.",
  grant_cost: "Usage cost covered by credit grants.",
  allowance_cost: "Usage cost covered by allowance units.",
};
export function panelSpan(panel: AnalyticsPanel): 1 | 2 | 3 {
  return panel.span ?? (panel.wide ? 3 : 1);
}
export const PANEL_HEIGHTS = { compact: 210, standard: 280, tall: 380 };
export function timeBucketStart(
  at: number,
  granularity: AnalyticsResult["granularity"],
): number {
  const date = new Date(at);
  date.setUTCMinutes(0, 0, 0);
  if (granularity !== "hour") date.setUTCHours(0);
  if (granularity === "week")
    date.setUTCDate(date.getUTCDate() - ((date.getUTCDay() + 6) % 7));
  if (granularity === "month") date.setUTCDate(1);
  return date.getTime();
}
export function bucketBounds(
  bucket: string,
  granularity: AnalyticsResult["granularity"],
) {
  const start = new Date(bucket);
  const end = new Date(bucket);
  if (granularity === "month") end.setUTCMonth(end.getUTCMonth() + 1);
  else if (granularity === "week") end.setUTCDate(end.getUTCDate() + 7);
  else if (granularity === "day") end.setUTCDate(end.getUTCDate() + 1);
  else end.setUTCHours(end.getUTCHours() + 1);
  return { start: start.getTime(), end: end.getTime() };
}
export function partialBucket(bucket: string, data: AnalyticsResult): boolean {
  const bounds = bucketBounds(bucket, data.granularity);
  return (
    bounds.start < Date.parse(data.window.from) ||
    bounds.end > Date.parse(data.window.to)
  );
}
export const BREAKDOWN_LABELS = {
  service: "Service",
  user: "Acting user",
  owner: "Billing account",
  credential_class: "Credential class",
};
export const TEMPLATE_COPY: Record<
  AnalyticsLayout,
  { name: string; reference: string; description: string; recommended: boolean }
> = {
  overview: {
    name: "Overview",
    reference: "GA inspired",
    description:
      "The big picture. Follow spend, spot trends, and see what drives usage.",
    recommended: false,
  },
  operations: {
    name: "Operations",
    reference: "Grafana inspired",
    description:
      "Your own control room. Arrange the metrics that matter in one place.",
    recommended: true,
  },
  explorer: {
    name: "Explorer",
    reference: "PostHog inspired",
    description:
      "Ask a focused question. Choose a measure, break it down, and inspect the details.",
    recommended: false,
  },
};
export const EMPTY_FILTERS: AnalyticsFilters = {
  period: "7d",
  from: null,
  to: null,
  services: [],
  actors: [],
  owners: [],
};
export function newPanel(
  overrides: Partial<AnalyticsPanel> = {},
): AnalyticsPanel {
  return {
    id: crypto.randomUUID(),
    title: "Service cost",
    chart: "bar",
    measure: "cost",
    metric: "tokens",
    breakdown: "service",
    top: 5,
    wide: false,
    ...overrides,
  };
}
export function duplicatePanel(panel: AnalyticsPanel): AnalyticsPanel {
  let title = "";
  for (const character of panel.title) {
    if (new TextEncoder().encode(`${title}${character} copy`).length > 100)
      break;
    title += character;
  }
  return { ...panel, id: crypto.randomUUID(), title: `${title} copy` };
}
export function newView(
  layout: AnalyticsLayout = "operations",
  filters: AnalyticsFilters = EMPTY_FILTERS,
): AnalyticsView {
  const panels =
    layout === "explorer"
      ? [
          newPanel({
            title: "Explore service usage",
            chart: "bar",
            top: 10,
            wide: true,
          }),
        ]
      : layout === "operations"
        ? [
            newPanel({
              title: "Request traffic",
              measure: "requests",
              chart: "line",
            }),
            newPanel({ title: "Cost & traffic", chart: "combo" }),
            newPanel({
              title: "Service spend",
              measure: "cost",
              chart: "line",
            }),
            newPanel({
              title: "Top billing accounts",
              breakdown: "owner",
              chart: "bar",
              top: 10,
            }),
            newPanel({
              title: "Token distribution",
              measure: "total_tokens",
              chart: "pie",
            }),
            newPanel({
              title: "Active user traffic",
              measure: "requests",
              breakdown: "user",
              chart: "bar",
            }),
          ]
        : [
            newPanel({ title: "Spend over time", chart: "combo", wide: true }),
            newPanel({ title: "Service mix", chart: "pie" }),
            newPanel({ title: "Leading services", chart: "bar" }),
          ];
  return {
    id: crypto.randomUUID(),
    name: TEMPLATE_COPY[layout].name,
    layout,
    filters: structuredClone(filters),
    panels,
  };
}
export function filterError(filters: AnalyticsFilters): string | null {
  return filters.period === "custom"
    ? usageRangeError(filters.from ?? undefined, filters.to ?? undefined)
    : null;
}
export function analyticsPath(
  filters: AnalyticsFilters,
  panel: AnalyticsPanel,
): string {
  const query = new URLSearchParams();
  if (filters.period === "custom") {
    if (filters.from) query.set("from", filters.from);
    if (filters.to) query.set("to", filters.to);
  } else query.set("period", filters.period);
  for (const key of ["services", "actors", "owners"] as const) {
    if (filters[key].length)
      query.set(key, [...new Set(filters[key])].sort().join(","));
  }
  query.set("measure", panel.measure);
  query.set("metric", panel.metric);
  query.set("breakdown", panel.breakdown);
  query.set("top", String(panel.top));
  if (panel.interval && panel.interval !== "auto")
    query.set("interval", panel.interval);
  return `/admin/usage/analytics?${query}`;
}
export function formatAnalyticsValue(
  value: number | null,
  unit: string,
  compact = false,
): string {
  if (value === null) return "Unknown";
  const amount = unit === "microcredits" ? value / 1_000_000 : value;
  return new Intl.NumberFormat(undefined, {
    notation: compact ? "compact" : "standard",
    maximumFractionDigits: compact ? 1 : unit === "microcredits" ? 6 : 0,
  }).format(amount);
}
export function unitLabel(unit: string): string {
  return unit === "microcredits" ? "credits" : unit.replaceAll("_", " ");
}
export function bucketLabel(
  bucket: string,
  granularity: AnalyticsResult["granularity"],
  full = false,
): string {
  return new Intl.DateTimeFormat(undefined, {
    timeZone: "UTC",
    month: "short",
    ...(granularity !== "month" ? { day: "numeric" as const } : {}),
    ...(granularity === "hour"
      ? { hour: "2-digit", minute: "2-digit", hourCycle: "h23" as const }
      : {}),
    ...(full ? { year: "numeric" } : {}),
  }).format(new Date(bucket));
}
