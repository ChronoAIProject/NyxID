import type { BillingUsageRow } from "@/schemas/billing";
import { decimalCredits, exactCredits, parseCredits } from "./credits";

export interface ServiceUsageSummary {
  /** Recorded calls; token-metered rows report `requests: 0`, so count events. */
  readonly calls: number;
  /** Metered quantity per unit, largest first. */
  readonly quantities: readonly { metric: string; quantity: number }[];
  /** Total NyxID credits charged; null when a charged row has no settled amount. */
  readonly charged: string | null;
  readonly wallet: string;
  readonly grant: string;
  readonly allowance: string;
  /** Any row was chargeable; false means metering only. */
  readonly billable: boolean;
  readonly agents: readonly { name: string | null; calls: number }[];
}

function sum(values: readonly (string | null)[]): string {
  return decimalCredits(
    values.reduce((total, value) => total + parseCredits(value ?? "0"), 0n),
  );
}

/**
 * `/billing/usage` records usage by the proxy slug the caller used, for the
 * caller's own billing account. Connections sharing a slug share these rows.
 */
export function serviceUsageSummary(
  rows: readonly BillingUsageRow[],
  slug: string,
): ServiceUsageSummary | null {
  const matching = rows.filter((row) => row.service_slug === slug);
  if (!matching.length) return null;
  const quantities = new Map<string, number>();
  const agents = new Map<string | null, number>();
  for (const row of matching) {
    quantities.set(
      row.metric,
      (quantities.get(row.metric) ?? 0) + row.quantity,
    );
    const name = row.api_key_id
      ? (row.api_key_name ?? "Unnamed agent key")
      : null;
    agents.set(name, (agents.get(name) ?? 0) + row.events);
  }
  const estimates = matching.map((row) =>
    row.billable
      ? exactCredits(row.estimated_credits, row.estimated_credits_micros)
      : "0",
  );
  return {
    calls: matching.reduce((total, row) => total + row.events, 0),
    quantities: [...quantities]
      .map(([metric, quantity]) => ({ metric, quantity }))
      .sort((a, b) => b.quantity - a.quantity),
    charged: estimates.includes(null) ? null : sum(estimates),
    wallet: sum(
      matching.map((row) =>
        exactCredits(row.wallet_credits, row.wallet_credits_micros),
      ),
    ),
    grant: sum(
      matching.map((row) =>
        exactCredits(row.grant_credits, row.grant_credits_micros),
      ),
    ),
    allowance: sum(
      matching.map((row) =>
        exactCredits(row.allowance_credits, row.allowance_credits_micros),
      ),
    ),
    billable: matching.some((row) => row.billable),
    agents: [...agents]
      .map(([name, calls]) => ({ name, calls }))
      .sort((a, b) => b.calls - a.calls),
  };
}

export interface ServiceUsageDay {
  /** UTC calendar day, `YYYY-MM-DD`. */
  readonly day: string;
  readonly calls: number;
  readonly quantities: readonly { metric: string; quantity: number }[];
  /** NyxID credits charged that day; null when a charged row is unsettled. */
  readonly charged: string | null;
}

const PERIOD_DAYS: Record<string, number> = { "7d": 7, "30d": 30, "90d": 90 };

/**
 * Zero-filled UTC daily series from `bucket=day` rows, oldest first. Null when
 * the period has no daily view or the server did not split rows by day.
 */
export function serviceUsageDaily(
  rows: readonly BillingUsageRow[],
  slug: string,
  period: string,
  now = new Date(),
): ServiceUsageDay[] | null {
  const days = PERIOD_DAYS[period];
  const matching = rows.filter((row) => row.service_slug === slug);
  // Older servers ignore `bucket=day`; any undated row means no daily split.
  if (!days || rows.some((row) => !row.day)) return null;
  const byDay = new Map<string, BillingUsageRow[]>();
  for (const row of matching) {
    const key = row.day!.slice(0, 10);
    byDay.set(key, [...(byDay.get(key) ?? []), row]);
  }
  const today = Date.UTC(
    now.getUTCFullYear(),
    now.getUTCMonth(),
    now.getUTCDate(),
  );
  return Array.from({ length: days }, (_, i) => {
    const day = new Date(today - (days - 1 - i) * 86_400_000)
      .toISOString()
      .slice(0, 10);
    const summary = serviceUsageSummary(byDay.get(day) ?? [], slug);
    return {
      day,
      calls: summary?.calls ?? 0,
      quantities: summary?.quantities ?? [],
      charged: summary ? summary.charged : "0",
    };
  });
}
