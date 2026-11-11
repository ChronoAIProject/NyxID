import type {
  NyxbotModelUsage,
  NyxbotModelStats,
  NyxbotUsageService,
} from "@/schemas/nyxbot-model-usage";
import type { AnalyticsResult } from "@/schemas/usage-analytics";
export const modelLabel = (model: {
  model: string | null;
  is_other: boolean;
}) => (model.is_other ? "Other models" : (model.model ?? "Unknown model"));
export const seriesColor = (index: number, other: boolean) =>
  other ? "var(--analytics-other)" : `var(--analytics-series-${index + 1})`;
const same = (
  a: { model: string | null; is_other: boolean },
  b: { model: string | null; is_other: boolean },
) => a.model === b.model && a.is_other === b.is_other;
export const costFields = [
  "gross_cost",
  "wallet_cost",
  "grant_cost",
  "allowance_cost",
] as const;
export const tokenFields = [
  "total_tokens",
  "prompt_tokens",
  "completion_tokens",
  "cached_tokens",
  "cache_creation_tokens",
  "audio_input_tokens",
  "audio_output_tokens",
  "image_input_tokens",
  "image_output_tokens",
] as const;
export const modelMeasureLabels = {
  gross_cost: "Gross credits",
  wallet_cost: "Wallet credits",
  grant_cost: "Grant credits",
  allowance_cost: "Allowance credits",
  requests: "Requests",
  total_tokens: "Total tokens",
  prompt_tokens: "Input tokens",
  completion_tokens: "Output tokens",
  cached_tokens: "Cache read tokens",
  cache_creation_tokens: "Cache write tokens",
  audio_input_tokens: "Audio input tokens",
  audio_output_tokens: "Audio output tokens",
  image_input_tokens: "Image input tokens",
  image_output_tokens: "Image output tokens",
};
export type Measure =
  | (typeof costFields)[number]
  | (typeof tokenFields)[number]
  | "requests";
export function modelDailyPoints(data: NyxbotModelUsage, measure: Measure) {
  const points: { day: string; [key: string]: string | number | null }[] = [];
  const end = Date.parse(data.window.to);
  for (
    let at = Math.floor(Date.parse(data.window.from) / 86400000) * 86400000;
    at < end;
    at += 86400000
  ) {
    const day = new Date(at).toISOString().slice(0, 10);
    const point: (typeof points)[number] = { day };
    data.models.forEach((model, index) => {
      const row = data.daily.find(
        (d) => d.day.slice(0, 10) === day && same(d, model),
      );
      const value = row ? row.usage[measure] : 0;
      const partial =
        row &&
        row.usage.unknown_cost_events > 0 &&
        measure !== "grant_cost" &&
        costFields.includes(measure as (typeof costFields)[number]);
      point[`s${index}`] = value === null ? null : Number(value);
      point[`exact${index}`] =
        value === null ? "Unavailable" : `${partial ? "≥ " : ""}${value}`;
    });
    points.push(point);
  }
  return points;
}
export type CostField =
  | "gross_cost"
  | "wallet_cost"
  | "grant_cost"
  | "allowance_cost";
export function modelCost(usage: NyxbotModelStats, field: CostField) {
  const value = usage[field];
  const partial = usage.unknown_cost_events > 0 && field !== "grant_cost";
  return value === null ? "Unavailable" : `${partial ? "≥ " : ""}${value}`;
}
type BreakdownSlice = {
  id: string | null;
  label: string;
  usage: NyxbotModelStats;
  isOther: boolean;
};
/** A NyxBot breakdown in the dashboard's analytics shape, so the same panel renders it. */
function breakdownAnalytics(
  data: NyxbotModelUsage,
  slices: BreakdownSlice[],
  measure: "cost" | "requests",
): AnalyticsResult {
  const cost = measure === "cost";
  return {
    window: data.window,
    freshness: data.freshness,
    granularity: "day",
    unit: cost ? "microcredits" : "requests",
    total: cost ? null : data.totals.requests,
    ...(cost ? { exact_total: data.totals.gross_cost } : {}),
    totals: {
      ...data.totals,
      // Cost caveats only describe cost panels.
      ...(cost ? {} : { unknown_cost_events: 0, legacy_cost_events: 0 }),
      quantities: {},
      unique_users: 0,
      unique_services: 0,
      gross_cost_micros: null,
      wallet_cost_micros: null,
      grant_cost_micros: null,
      allowance_cost_micros: null,
    },
    points: [],
    series: [],
    slices: slices.map(({ id, label, usage, isOther }) => ({
      id,
      label,
      ...(cost
        ? { value: null, exact_value: usage.gross_cost }
        : { value: usage.requests }),
      unknown_cost_events: cost ? usage.unknown_cost_events : 0,
      is_other: isOther,
    })),
  };
}
export function accountAnalytics(data: NyxbotModelUsage): AnalyticsResult {
  return breakdownAnalytics(
    data,
    [
      ...data.accounts.map(({ account, usage }) => ({
        id: account.id,
        label: account.display_name,
        usage,
        isOther: false,
      })),
      ...(data.other_accounts
        ? [
            {
              id: null,
              label: "Other",
              usage: data.other_accounts.usage,
              isOther: true,
            },
          ]
        : []),
    ],
    "cost",
  );
}
export function serviceLabel(row: NyxbotUsageService): string {
  if (row.is_other || !row.service) return "Other";
  const { service_name, service_slug, service_id } = row.service;
  return service_name !== "Unknown service"
    ? service_name
    : (service_slug ?? service_id ?? service_name);
}
export function serviceAnalytics(data: NyxbotModelUsage): AnalyticsResult {
  return breakdownAnalytics(
    data,
    data.services.map((row) => ({
      id: row.service?.service_id ?? row.service?.service_slug ?? null,
      label: serviceLabel(row),
      usage: row.usage,
      isOther: row.is_other,
    })),
    "requests",
  );
}
