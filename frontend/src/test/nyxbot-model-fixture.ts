import {
  modelStatsSchema,
  type NyxbotModelUsage,
  type NyxbotModelStats,
  type NyxbotUsageModel,
} from "@/schemas/nyxbot-model-usage";
import { decimalCredits, parseCredits } from "@/lib/credits";
import { usageStats } from "./admin-usage-fixture";
export function modelStats(
  overrides: Partial<NyxbotModelStats> = {},
): NyxbotModelStats {
  return modelStatsSchema.parse({
    ...usageStats(),
    gross_cost: "2.000000000001",
    wallet_cost: "1.000000000001",
    grant_cost: "0.5",
    allowance_cost: "0.5",
    ...overrides,
  });
}
const costFields = [
  "gross_cost",
  "wallet_cost",
  "grant_cost",
  "allowance_cost",
] as const;
export function sumModelStats(rows: NyxbotModelStats[]): NyxbotModelStats {
  const result = modelStats();
  for (const key of Object.keys(result) as (keyof NyxbotModelStats)[]) {
    if ((costFields as readonly string[]).includes(key)) continue;
    (result[key] as number) = rows.reduce(
      (sum, row) => sum + (row[key] as number),
      0,
    );
  }
  for (const key of costFields) {
    const known = rows.map((r) => r[key]).filter((v) => v !== null);
    result[key] = known.length
      ? decimalCredits(known.reduce((n, v) => n + parseCredits(v), 0n))
      : null;
  }
  return result;
}
export function nyxbotModelFixture(
  from = "2026-10-10T00:00:00Z",
  to = "2026-10-17T00:00:00Z",
  filter?: string | null,
): NyxbotModelUsage {
  const names = ["claude-sonnet-4-5", "gpt-5.4", null];
  const identities = [
    {
      id: "alice",
      display_name: "Alice Research",
      email: "alice@example.test",
      user_type: "person",
    },
    {
      id: "org",
      display_name: "Observatory team",
      email: null,
      user_type: "org",
    },
  ];
  const rows: {
    day: string;
    model: string | null;
    owner: number;
    usage: NyxbotModelStats;
  }[] = [];
  for (
    let day = 0;
    day < 7 && Date.parse(from) + day * 86400000 < Date.parse(to);
    day++
  ) {
    names.forEach((model, m) => {
      if (filter !== undefined && filter !== model) return;
      const requests = (m + 1) * (day + 1);
      const gross =
        m === 0
          ? String(day + 1)
          : m === 1
            ? day === 0
              ? "0.000000000001"
              : "0"
            : null;
      const unknown = m === 2;
      rows.push({
        day: new Date(Date.parse(from) + day * 86400000).toISOString(),
        model,
        owner: day % 2,
        usage: modelStats({
          requests,
          events: requests,
          prompt_tokens: requests * 1000,
          completion_tokens: requests * 100,
          cached_tokens: requests * 300,
          cache_creation_tokens: requests * 50,
          total_tokens: requests * 1100,
          audio_input_tokens: m === 1 ? requests * 10 : 0,
          audio_output_tokens: 0,
          image_input_tokens: m === 1 ? requests * 5 : 0,
          image_output_tokens: 0,
          gross_cost: gross,
          wallet_cost:
            m === 0 ? decimalCredits(parseCredits(gross!) / 2n) : gross,
          grant_cost: m === 0 ? decimalCredits(parseCredits(gross!) / 4n) : "0",
          allowance_cost:
            m === 0
              ? decimalCredits(parseCredits(gross!) / 4n)
              : unknown
                ? null
                : "0",
          exact_cost_events: unknown ? 0 : requests,
          legacy_cost_events: unknown ? requests : 0,
          unknown_cost_events: unknown ? requests : 0,
        }),
      });
    });
  }
  const groupedModels = (selected: typeof rows): NyxbotUsageModel[] =>
    names
      .filter((model) => selected.some((r) => r.model === model))
      .map((model) => ({
        model,
        is_other: false,
        groups: 1,
        usage: sumModelStats(
          selected.filter((r) => r.model === model).map((r) => r.usage),
        ),
      }));
  const totals = sumModelStats(rows.map((r) => r.usage));
  return {
    window: { from, to, period: "custom" },
    freshness: { validated: true, tail_rows: 3, rolled_up_through: to },
    totals,
    models: groupedModels(rows),
    daily: rows.map((row) => ({
      day: row.day,
      model: row.model,
      is_other: false,
      groups: 1,
      usage: row.usage,
    })),
    accounts: identities.flatMap((account, owner) => {
      const selected = rows.filter((r) => r.owner === owner);
      return selected.length
        ? [
            {
              account,
              usage: sumModelStats(selected.map((r) => r.usage)),
              models: groupedModels(selected),
              other_model_count: 0,
            },
          ]
        : [];
    }),
    other_accounts: null,
    excluded: [],
  };
}
