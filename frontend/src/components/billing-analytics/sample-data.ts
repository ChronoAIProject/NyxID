import type {
  AnalyticsFilters,
  AnalyticsPanel,
  AnalyticsResult,
} from "@/schemas/usage-analytics";
import type { SampleOptions } from "./controls";
import { usageStatsSchema } from "@/schemas/admin-usage";
import { bucketBounds, timeBucketStart } from "@/lib/usage-analytics";
import { credentialClassLabel } from "@/lib/billing-units";

const uid = (n: number) =>
  `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const services = [
  "OpenAI",
  "Anthropic",
  "Gemini",
  "xAI",
  "OpenClaw",
  "Oracle",
  "Mistral",
  "Cohere",
  "DeepSeek",
  "Stability",
  "Perplexity",
  "ElevenLabs",
];
const people = [
  "Alex Chen",
  "Maya Patel",
  "Sam Rivera",
  "Jordan Lee",
  "Noor Hassan",
  "Chris Park",
  "Taylor Kim",
  "Robin Chen",
];
const orgs = ["Platform Engineering", "Product Studio", "Research Lab"];
export const SAMPLE_OPTIONS: SampleOptions = {
  services: services.map((label, i) => ({
    id: uid(i + 1),
    label,
    detail: label.toLowerCase().replaceAll(" ", "-"),
  })),
  actors: people.map((label, i) => ({
    id: uid(i + 101),
    label,
    detail: `${label.toLowerCase().split(" ")[0]}@example.test`,
  })),
  owners: orgs.map((label, i) => ({ id: uid(i + 201), label })),
};
const END = Date.parse("2026-09-23T12:00:00Z");
const HOUR = 3_600_000;
const EVENTS = Array.from({ length: 31 * 24 }, (_, hour) =>
  services.map((_, service) => {
    const actor = (hour + service * 3) % people.length;
    const owner = (hour + service) % orgs.length;
    const day = Math.floor(hour / 24);
    const wave =
      0.55 +
      0.45 * Math.sin((((hour % 24) - 6 - service * 1.4) * Math.PI) / 12);
    const weekly =
      0.8 + 0.28 * Math.sin(((day + service * 0.65) * Math.PI) / 3.5);
    const activity = 1 + 0.3 * Math.cos(day * 0.9 + service * 1.7);
    const requests = Math.round(
      (720 * 0.79 ** service + 18) * (0.65 + wave) * weekly * activity,
    );
    const cost = requests * (1700 + service * 713);
    return {
      time: END - (31 * 24 - hour) * HOUR,
      service: uid(service + 1),
      actor: uid(actor + 101),
      owner: uid(owner + 201),
      credentialClass: service % 2 ? "user_owned" : "nyxid_managed_master",
      requests,
      tokens: requests * (720 + service * 41),
      cost,
    };
  }),
).flat();
export function sampleAnalytics(
  filters: AnalyticsFilters,
  panel: AnalyticsPanel,
): AnalyticsResult {
  const end = filters.period === "custom" ? Date.parse(filters.to!) : END;
  const start =
    filters.period === "custom"
      ? Date.parse(filters.from!)
      : end - { "24h": 24, "7d": 168, "30d": 720 }[filters.period] * HOUR;
  const granularity =
    !panel.interval || panel.interval === "auto"
      ? end - start <= 48 * HOUR
        ? "hour"
        : "day"
      : panel.interval;
  const rows = EVENTS.filter(
    (row) =>
      row.time >= start &&
      row.time < end &&
      (!filters.services.length || filters.services.includes(row.service)) &&
      (!filters.actors.length || filters.actors.includes(row.actor)) &&
      (!filters.owners.length || filters.owners.includes(row.owner)),
  );
  const stats = usageStatsSchema.parse({
    requests: 0,
    events: 0,
    quantities: {},
    prompt_tokens: 0,
    completion_tokens: 0,
    cached_tokens: 0,
    cache_creation_tokens: 0,
    total_tokens: 0,
    gross_cost_micros: 0,
    wallet_cost_micros: 0,
    grant_cost_micros: 0,
    allowance_cost_micros: 0,
    exact_cost_events: 0,
    legacy_cost_events: 0,
    unknown_cost_events: 0,
    unique_users: new Set(rows.map((r) => r.actor)).size,
    unique_services: new Set(rows.map((r) => r.service)).size,
  });
  const unit =
    panel.measure.includes("cost") && !panel.measure.endsWith("_events")
      ? "microcredits"
      : panel.measure === "quantity"
        ? panel.metric
        : panel.measure.endsWith("_tokens")
          ? "tokens"
          : panel.measure === "events" || panel.measure.endsWith("_events")
            ? "events"
            : "requests";
  const points: AnalyticsResult["points"] = [];
  for (
    let at = timeBucketStart(start, granularity);
    at < end;
    at = bucketBounds(new Date(at).toISOString(), granularity).end
  )
    points.push({
      bucket: new Date(at).toISOString(),
      value: 0,
      requests: 0,
      unknown_cost_events: 0,
    });
  const buckets = new Map(
    points.map((point) => [Date.parse(point.bucket), point]),
  );
  const groups = new Map<string, number>();
  const groupedPoints = new Map<
    string,
    Map<number, { value: number; requests: number }>
  >();
  for (const row of rows) {
    const input = Math.round(row.tokens * 0.78),
      output = row.tokens - input;
    const quantities: Record<string, number> = {
      tokens: row.tokens,
      requests: row.requests,
      bytes: row.requests * 1024,
      input_tokens: input,
      output_tokens: output,
      cache_read_tokens: Math.round(input * 0.2),
      cache_write_tokens: Math.round(input * 0.05),
      images: row.service === uid(10) ? row.requests : 0,
    };
    const wallet = Math.floor(row.cost * 0.68),
      grants = Math.floor(row.cost * 0.21),
      allowance = row.cost - wallet - grants;
    const providerTokens: Record<string, number> = {
      total_tokens: row.tokens,
      prompt_tokens: input,
      completion_tokens: output,
      cached_tokens: quantities.cache_read_tokens!,
      cache_creation_tokens: quantities.cache_write_tokens!,
    };
    const eventCounts: Record<string, number> = {
      events: row.requests,
      exact_cost_events: row.requests,
      legacy_cost_events: 0,
      unknown_cost_events: 0,
    };
    const amount =
      panel.measure === "cost"
        ? row.cost
        : panel.measure === "wallet_cost"
          ? wallet
          : panel.measure === "grant_cost"
            ? grants
            : panel.measure === "allowance_cost"
              ? allowance
              : panel.measure === "requests"
                ? row.requests
                : panel.measure.endsWith("_tokens")
                  ? providerTokens[panel.measure]!
                  : panel.measure === "events" ||
                      panel.measure.endsWith("_events")
                    ? eventCounts[panel.measure]!
                    : quantities[panel.metric]!;
    const group =
      panel.breakdown === "service"
        ? row.service
        : panel.breakdown === "user"
          ? row.actor
          : panel.breakdown === "credential_class"
            ? row.credentialClass
            : row.owner;
    groups.set(group, (groups.get(group) ?? 0) + amount);
    const byTime =
      groupedPoints.get(group) ??
      new Map<number, { value: number; requests: number }>();
    const at = timeBucketStart(row.time, granularity);
    const previous = byTime.get(at) ?? { value: 0, requests: 0 };
    byTime.set(at, {
      value: previous.value + amount,
      requests: previous.requests + row.requests,
    });
    groupedPoints.set(group, byTime);
    const bucket = buckets.get(at)!;
    bucket.value! += amount;
    bucket.requests += row.requests;
    stats.requests += row.requests;
    stats.events += row.requests;
    stats.exact_cost_events += row.requests;
    stats.prompt_tokens += input;
    stats.completion_tokens += output;
    stats.total_tokens += row.tokens;
    stats.cached_tokens += quantities.cache_read_tokens!;
    stats.cache_creation_tokens += quantities.cache_write_tokens!;
    stats.gross_cost_micros! += row.cost;
    stats.wallet_cost_micros! += wallet;
    stats.grant_cost_micros! += grants;
    stats.allowance_cost_micros! += allowance;
    for (const [metric, value] of Object.entries(quantities))
      stats.quantities[metric] = (stats.quantities[metric] ?? 0) + value;
  }
  const total = points.reduce((sum, point) => sum + point.value!, 0);
  const names =
    panel.breakdown === "credential_class"
      ? [
          ...["user_owned", "nyxid_managed_master"].map((id) => ({
            id,
            label: credentialClassLabel(id),
          })),
        ]
      : SAMPLE_OPTIONS[
          panel.breakdown === "service"
            ? "services"
            : panel.breakdown === "user"
              ? "actors"
              : "owners"
        ];
  const sorted = [...groups].sort(
    (a, b) => b[1] - a[1] || a[0].localeCompare(b[0]),
  );
  const slices: AnalyticsResult["slices"] =
    panel.top === 0
      ? rows.length
        ? [
            {
              id: null,
              label: "All usage",
              value: total,
              unknown_cost_events: 0,
              is_other: false,
            },
          ]
        : []
      : sorted.slice(0, panel.top).map(([id, value]) => ({
          id,
          label: names.find((entry) => entry.id === id)!.label,
          value,
          unknown_cost_events: 0,
          is_other: false,
        }));
  if (panel.top > 0 && sorted.length > panel.top)
    slices.push({
      id: null,
      label: "Other",
      value: sorted.slice(panel.top).reduce((sum, [, value]) => sum + value, 0),
      unknown_cost_events: 0,
      is_other: true,
    });
  const seriesFor = (label: string, ids: string[], is_other = false) => ({
    label,
    is_other,
    points: points.map((point) => ({
      ...point,
      value: ids.reduce(
        (sum, id) =>
          sum +
          (groupedPoints.get(id)?.get(Date.parse(point.bucket))?.value ?? 0),
        0,
      ),
      requests: ids.reduce(
        (sum, id) =>
          sum +
          (groupedPoints.get(id)?.get(Date.parse(point.bucket))?.requests ?? 0),
        0,
      ),
    })),
  });
  const series =
    panel.top === 0
      ? [
          seriesFor(
            "All usage",
            sorted.map(([id]) => id),
          ),
        ]
      : sorted
          .slice(0, panel.top)
          .map(([id]) =>
            seriesFor(names.find((entry) => entry.id === id)!.label, [id]),
          );
  if (panel.top > 0 && sorted.length > panel.top)
    series.push(
      seriesFor(
        "Other",
        sorted.slice(panel.top).map(([id]) => id),
        true,
      ),
    );
  return {
    window: {
      from: new Date(start).toISOString(),
      to: new Date(end).toISOString(),
      period: filters.period,
    },
    freshness: {
      rolled_up_through: new Date(END).toISOString(),
      tail_rows: 0,
      validated: true,
    },
    granularity,
    unit,
    total,
    totals: stats,
    points,
    slices,
    series,
  };
}
