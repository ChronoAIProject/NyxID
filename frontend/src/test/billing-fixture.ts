import type {
  BillingUsageResponse,
  BillingUsageRow,
  BillingWalletResponse,
} from "@/schemas/billing";
import type {
  CreditGrant,
  UserAllowanceBalance,
} from "@/schemas/billing-credits";
import type { ApiKeyUsage } from "@/types/api";

export function billingAgentUsage(
  overrides: Partial<ApiKeyUsage> = {},
): ApiKeyUsage {
  return {
    api_key_id: "research-agent",
    api_key_name: "Research agent",
    platform: null,
    request_count: 29,
    success_count: 27,
    error_count: 2,
    error_rate: 2 / 29,
    last_used_at: "2026-10-07T00:00:00Z",
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
    reported_cost: null,
    top_services: [],
    daily_buckets: [3, 0, 8, 2, 5, 1, 10].map((requests, index) => ({
      date: `2026-10-${String(index + 1).padStart(2, "0")}`,
      request_count: requests,
      error_count: index === 2 || index === 4 ? 1 : 0,
    })),
    ...overrides,
  };
}

export const billingCatalog = [
  { slug: "example-llm", name: "Example LLM", inference: null },
  { slug: "free-service", name: "Free service", inference: null },
  { slug: "unused-service", name: "Unused service", inference: null },
];
export function billingWallet(
  overrides: Partial<BillingWalletResponse> = {},
): BillingWalletResponse {
  return {
    owner_id: "test-user",
    plan_kind: "prepaid",
    collection_state: "good",
    balance_credits: 100,
    reserved_credits: 2,
    pending_lago_debits: 3,
    pending_topup_expiry_credits: 0,
    available_credits: 95,
    available_with_overdraft_credits: 95,
    has_payment_instrument: false,
    overdraft_cap_credits: 0,
    suspended: false,
    lago_customer_id: "customer",
    lago_wallet_id: "wallet",
    balance_synced_at: "2026-09-25T00:00:00Z",
    created_at: "2026-09-01T00:00:00Z",
    updated_at: "2026-09-25T00:00:00Z",
    created: false,
    ...overrides,
  };
}
export function billingGrant(
  overrides: Partial<CreditGrant> = {},
): CreditGrant {
  return {
    id: "grant",
    batch_id: "batch",
    recipient_user_id: "test-user",
    activation_state: "active",
    target_kind: "selected_users",
    amount_credits: 10,
    amount_micros: 10_000_000,
    remaining_micros: 2_000_000,
    reserved_micros: 500_000,
    scope: { all_services: true, service_ids: [], service_slugs: [] },
    granted_by: "admin",
    status: "active",
    created_at: "2026-09-01T00:00:00Z",
    updated_at: "2026-09-25T00:00:00Z",
    expires_at: "2026-10-01T00:00:00Z",
    ...overrides,
  };
}
export function billingAllowance(
  metric: UserAllowanceBalance["allowance"]["metric"] = "input_tokens",
  overrides: Partial<UserAllowanceBalance> = {},
): UserAllowanceBalance {
  return {
    allowance: {
      id: metric,
      service_id: "service",
      service_slug: "example-llm",
      metric,
      quantity: 1000,
      recurrence: "daily",
      target_kind: "selected_users",
      target_user_ids: ["test-user"],
      is_active: true,
      created_by: "admin",
      created_at: "2026-09-01T00:00:00Z",
      updated_at: "2026-09-25T00:00:00Z",
    },
    period_start: "2026-09-25T00:00:00Z",
    period_end: "2026-09-26T00:00:00Z",
    consumed_quantity: 100,
    reserved_quantity: 100,
    remaining_quantity: 800,
    ...overrides,
  };
}

export function billingExpandedAllowances(): UserAllowanceBalance[] {
  const metrics = [
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "images",
  ] as const;
  return metrics.flatMap((metric) => [
    billingAllowance(metric, {
      allowance: {
        ...billingAllowance(metric).allowance,
        id: `${metric}-small`,
        quantity: 1_000_000,
      },
      consumed_quantity: 1_000_000,
      reserved_quantity: 0,
      remaining_quantity: 0,
    }),
    billingAllowance(metric, {
      allowance: {
        ...billingAllowance(metric).allowance,
        id: `${metric}-large`,
        quantity: 100_000_000,
      },
      consumed_quantity: 4_213_490,
      reserved_quantity: 536_202,
      remaining_quantity: 95_250_308,
    }),
  ]);
}

export function billingExpandedRows(): BillingUsageRow[] {
  const metrics = [
    "tokens",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "images",
  ] as const;
  const quantities: Record<string, number> = {
    tokens: 8_975_496,
    input_tokens: 24_192_034,
    output_tokens: 2_814_729,
    cache_read_tokens: 837_541_120,
    cache_write_tokens: 0,
    images: 0,
  };
  const weights = new Map<string, number>();
  const cumulative = new Map<string, number>();
  for (let index = 0; index < 178; index++) {
    const metric = metrics[index % metrics.length]!;
    weights.set(metric, (weights.get(metric) ?? 0) + index + 1);
  }
  return Array.from({ length: 178 }, (_, index) => {
    const metric = metrics[index % metrics.length]!;
    const priorWeight = cumulative.get(metric) ?? 0;
    cumulative.set(metric, priorWeight + index + 1);
    const total = quantities[metric]!;
    const weight = weights.get(metric)!;
    return billingRow({
      metric: metrics[index % metrics.length]!,
      model: `Model ${(index % 11) + 1}`,
      api_key_id: `agent-${index % 4}`,
      api_key_name: `Agent ${(index % 4) + 1}`,
      quantity:
        Math.floor((total * (priorWeight + index + 1)) / weight) -
        Math.floor((total * priorWeight) / weight),
    });
  });
}
export function billingRow(
  overrides: Partial<BillingUsageRow> = {},
): BillingUsageRow {
  return {
    service_slug: "example-llm",
    metric: "tokens",
    lago_metric_code: "platform_tokens",
    layer: "platform",
    quantity: 2440,
    requests: 1,
    bytes: 0,
    events: 1,
    lago_acked: true,
    billable: true,
    estimated_credits_micros: 2440,
    wallet_credits_micros: 0,
    grant_credits_micros: 2440,
    allowance_credits_micros: 0,
    allowance_quantity: 0,
    ...overrides,
  };
}
export function billingUsage(
  rows: BillingUsageRow[] = [billingRow()],
): BillingUsageResponse {
  const sum = (field: keyof BillingUsageRow) =>
    rows.reduce(
      (n, row) =>
        n + (typeof row[field] === "number" ? (row[field] as number) : 0),
      0,
    );
  return {
    owner_id: "test-user",
    period: "30d",
    rows,
    totals: {
      quantity: sum("quantity"),
      requests: sum("requests"),
      bytes: sum("bytes"),
      events: sum("events"),
      estimated_credits_micros: sum("estimated_credits_micros"),
      wallet_credits_micros: sum("wallet_credits_micros"),
      grant_credits_micros: sum("grant_credits_micros"),
      allowance_credits_micros: sum("allowance_credits_micros"),
      allowance_quantity: sum("allowance_quantity"),
    },
    billing: {
      charging_enabled: true,
      lago_configured: true,
      source: "usage_meter",
      rates_are_approximate: true,
    },
  };
}
