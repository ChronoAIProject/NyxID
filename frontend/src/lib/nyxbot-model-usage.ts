import type {
  NyxbotModelUsage,
  NyxbotModelStats,
} from "@/schemas/nyxbot-model-usage";
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
