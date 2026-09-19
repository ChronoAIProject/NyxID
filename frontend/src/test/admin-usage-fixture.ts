import type { AdminUsageResponse, AdminUsageStats } from "@/types/admin";
export function usageStats(
  overrides: Partial<AdminUsageStats> = {},
): AdminUsageStats {
  return {
    requests: 7,
    events: 14,
    quantities: { input_tokens: 100, output_tokens: 20, images: 2 },
    prompt_tokens: 100,
    completion_tokens: 20,
    cached_tokens: 30,
    cache_creation_tokens: 5,
    total_tokens: 120,
    gross_cost_micros: 2_000_000,
    wallet_cost_micros: 1_000_000,
    grant_cost_micros: 500_000,
    allowance_cost_micros: 500_000,
    exact_cost_events: 14,
    legacy_cost_events: 0,
    unknown_cost_events: 0,
    unique_users: 1,
    unique_services: 1,
    ...overrides,
  };
}
export function usageFixture(): AdminUsageResponse {
  const user = {
    id: "11111111-1111-4111-8111-111111111111",
    display_name: "Alice",
    email: "alice@example.test",
    user_type: "person",
  };
  const service = {
    service_id: "service-1",
    service_slug: "llm-example",
    service_name: "Example model",
  };
  const lane = { ...usageStats(), credential_class: "nyxid_managed_master" };
  return {
    window: {
      from: "2026-09-18T00:00:00Z",
      to: "2026-09-19T00:00:00Z",
      period: "24h",
    },
    totals: usageStats(),
    by_service: [{ ...service, ...usageStats(), by_credential_class: [lane] }],
    by_credential_class: [lane],
    ranking: [
      {
        ...service,
        ...usageStats(),
        user,
        billing_owner: {
          id: "22222222-2222-4222-8222-222222222222",
          display_name: "Research team",
          email: "team@example.test",
          user_type: "org",
        },
      },
    ],
    ranking_total: 30,
    page: 1,
    per_page: 25,
    ranking_metric: "tokens",
    services: [service],
    selected_user: null,
  };
}
