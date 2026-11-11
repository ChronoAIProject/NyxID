import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi } from "@/i18n/use-ui-translation";
import { formatExactCredits, formatCompactCredits } from "./credits";
import { usageRangeError } from "@/schemas/admin-usage";
import type {
  AnalyticsFilters,
  AnalyticsLayout,
  AnalyticsPanel,
  AnalyticsResult,
  AnalyticsView,
} from "@/schemas/usage-analytics";

export const MEASURE_LABELS: Record<AnalyticsPanel["measure"], string> = {
  get cost() { return translateUi("Gross cost"); },
  get requests() { return translateUi("Requests"); },
  get events() { return translateUi("Billing events"); },
  get exact_cost_events() { return translateUi("Exact-cost events"); },
  get legacy_cost_events() { return translateUi("Legacy billing events"); },
  get unknown_cost_events() { return translateUi("Uncosted events"); },
  get total_tokens() { return translateUi("Total tokens"); },
  get prompt_tokens() { return translateUi("Input tokens"); },
  get completion_tokens() { return translateUi("Output tokens"); },
  get cached_tokens() { return translateUi("Cache-read tokens"); },
  get cache_creation_tokens() { return translateUi("Cache-write tokens"); },
  get audio_input_tokens() { return translateUi("Voice input (audio tokens)"); },
  get audio_output_tokens() { return translateUi("Voice output (audio tokens)"); },
  get quantity() { return translateUi("Billed units"); },
  get wallet_cost() { return translateUi("Wallet cost"); },
  get grant_cost() { return translateUi("Grant cost"); },
  get allowance_cost() { return translateUi("Allowance cost"); },
};
export const INTERVAL_LABELS = {
  get auto() { return translateUi("Auto"); },
  get hour() { return translateUi("Hourly"); },
  get day() { return translateUi("Daily"); },
  get week() { return translateUi("Weekly"); },
  get month() { return translateUi("Monthly"); },
};
export const MEASURE_DESCRIPTIONS: Record<AnalyticsPanel["measure"], string> = {
  get cost() { return translateUi("Total usage cost before grants and allowances, in credits."); },
  get requests() { return translateUi("Metered platform requests, counted once across billing components. Unmetered activity is not included."); },
  get events() { return translateUi("Metering records. One request can produce several billing events."); },
  get exact_cost_events() { return translateUi("Billing events with recorded settlement costs."); },
  get legacy_cost_events() { return translateUi("Historical billing events with estimated or unavailable cost. These can overlap uncosted events."); },
  get unknown_cost_events() { return translateUi("Billing events whose cost could not be determined. These leave gaps in cost charts."); },
  get total_tokens() { return translateUi("Provider-reported input + output tokens. Cache counts can overlap input; they are not added again."); },
  get prompt_tokens() { return translateUi("Provider-reported input tokens. Some providers include cached input here."); },
  get completion_tokens() { return translateUi("Provider-reported output tokens."); },
  get cached_tokens() { return translateUi("Provider-reported cached input reads. This can overlap input tokens."); },
  get cache_creation_tokens() { return translateUi("Provider-reported cache writes. Provider accounting varies; do not add this to total tokens."); },
  get audio_input_tokens() { return translateUi("Provider-reported audio input tokens from realtime voice and audio chat. Already included in input tokens; not priced separately."); },
  get audio_output_tokens() { return translateUi("Provider-reported audio output tokens from realtime voice and audio chat. Already included in output tokens; not priced separately."); },
  get quantity() { return translateUi("Sum of metered units in the selected billing metric. These can differ from provider-reported tokens."); },
  get wallet_cost() { return translateUi("Usage cost funded by purchased wallet credits."); },
  get grant_cost() { return translateUi("Usage cost covered by credit grants."); },
  get allowance_cost() { return translateUi("Usage cost covered by allowance units."); },
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
  get service() { return translateUi("Service"); },
  get model() { return translateUi("Model"); },
  get user() { return translateUi("Acting user"); },
  get owner() { return translateUi("Billing account"); },
  get credential_class() { return translateUi("Credential class"); },
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
              title: "Tokens by model",
              breakdown: "model",
              measure: "total_tokens",
              chart: "pie",
            }),
            newPanel({
              title: "Active user traffic",
              measure: "requests",
              breakdown: "user",
              chart: "bar",
            }),
            newPanel({
              title: "Voice output",
              measure: "audio_output_tokens",
              chart: "line",
              span: 2,
            }),
            newPanel({
              title: "Voice input",
              measure: "audio_input_tokens",
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
  value: string | number | null,
  unit: string,
  compact = false,
): string {
  if (value === null) return "Unknown";
  if (typeof value === "string")
    return compact ? formatCompactCredits(value) : formatExactCredits(value);
  const amount = unit === "microcredits" ? value / 1_000_000 : value;
  return new Intl.NumberFormat(getFormattingLocale(), {
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
  return new Intl.DateTimeFormat(getFormattingLocale(), {
    timeZone: "UTC",
    month: "short",
    ...(granularity !== "month" ? { day: "numeric" as const } : {}),
    ...(granularity === "hour"
      ? { hour: "2-digit", minute: "2-digit", hourCycle: "h23" as const }
      : {}),
    ...(full ? { year: "numeric" } : {}),
  }).format(new Date(bucket));
}

/** The decimal amount is authoritative; legacy integers are display fallback. */
export function analyticsAmount(item: {
  value: number | null;
  exact_value?: string | null;
}): string | number | null {
  return item.exact_value === undefined ? item.value : item.exact_value;
}
/** Explicit, lossy chart-coordinate boundary. Tables and tooltips retain strings. */
export function plotAmount(item: {
  value: number | null;
  exact_value?: string | null;
}): number | null {
  const value = analyticsAmount(item);
  return typeof value === "string" ? Number(value) * 1_000_000 : value;
}
