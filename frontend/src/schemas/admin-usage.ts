import { z } from "zod";
import { BILLING_METRICS } from "./billing-metrics";

const count = z.number().int().nonnegative();
export const usageStatsSchema = z.object({
  requests: count,
  events: count,
  // Keep future metrics: labels fall back to the raw metric string.
  quantities: z.record(z.string(), count),
  prompt_tokens: count,
  completion_tokens: count,
  cached_tokens: count,
  cache_creation_tokens: count,
  total_tokens: count,
  gross_cost_micros: count.nullable(),
  wallet_cost_micros: count.nullable(),
  grant_cost_micros: count.nullable(),
  allowance_cost_micros: count.nullable(),
  exact_cost_events: count,
  legacy_cost_events: count,
  unknown_cost_events: count,
  unique_users: count,
  unique_services: count,
});
export const usageIdentitySchema = z.object({
  id: z.string(),
  display_name: z.string(),
  email: z.string().nullable(),
  user_type: z.string(),
});
export const usageServiceIdentitySchema = z.object({
  service_id: z.string().nullable(),
  service_slug: z.string().nullable(),
  service_name: z.string(),
});
export const usageCredentialSchema = usageStatsSchema.extend({
  credential_class: z.string(),
});
export const usageServiceSchema = usageStatsSchema.extend({
  ...usageServiceIdentitySchema.shape,
  by_credential_class: z.array(usageCredentialSchema),
});
export const usageRankingSchema = usageStatsSchema.extend({
  ...usageServiceIdentitySchema.shape,
  user: usageIdentitySchema,
  billing_owner: usageIdentitySchema.nullable(),
});
export const adminUsageResponseSchema = z.object({
  freshness: z
    .object({
      rolled_up_through: z.iso.datetime({ offset: true }),
      tail_rows: count,
    })
    .optional(),
  window: z.object({
    from: z.iso.datetime({ offset: true }),
    to: z.iso.datetime({ offset: true }),
    period: z.string(),
  }),
  totals: usageStatsSchema,
  by_service: z.array(usageServiceSchema),
  by_credential_class: z.array(usageCredentialSchema),
  ranking: z.array(usageRankingSchema),
  ranking_total: count,
  page: count.positive(),
  per_page: count.positive().max(100),
  ranking_metric: z.string(),
  services: z.array(usageServiceIdentitySchema),
  selected_user: usageIdentitySchema.nullable(),
});
export const USAGE_SORTS = [
  "requests",
  "quantity",
  "cost",
  "total_tokens",
  "prompt_tokens",
  "completion_tokens",
  "cached_tokens",
  "cache_creation_tokens",
] as const;
export const adminUsageSearchSchema = z.object({
  period: z.enum(["24h", "7d", "30d", "custom"]).catch("24h"),
  from: z.string().optional().catch(undefined),
  to: z.string().optional().catch(undefined),
  user: z.uuid().optional().catch(undefined),
  service: z.string().max(256).optional().catch(undefined),
  sort: z.enum(USAGE_SORTS).catch("requests"),
  metric: z.enum(BILLING_METRICS).catch("tokens"),
  page: z.coerce
    .number()
    .int()
    .positive()
    .max(Number.MAX_SAFE_INTEGER)
    .catch(1),
  per_page: z.coerce.number().int().min(1).max(100).catch(25),
});
export function normalizeAdminUsageSearch(value: Record<string, unknown>) {
  return adminUsageSearchSchema.parse({
    ...value,
    period: value.period ?? (value.from || value.to ? "custom" : "24h"),
  });
}
export function usageRangeError(from?: string, to?: string): string | null {
  if (
    !from ||
    !to ||
    !z.iso.datetime({ offset: true }).safeParse(from).success ||
    !z.iso.datetime({ offset: true }).safeParse(to).success
  ) {
    return "Choose a start and end time.";
  }
  const duration = Date.parse(to) - Date.parse(from);
  return duration <= 0 || duration > 31 * 86_400_000
    ? "Choose a positive range of at most 31 days."
    : null;
}
