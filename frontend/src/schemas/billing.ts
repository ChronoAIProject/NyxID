import { z } from "zod";

export const BILLING_USAGE_PERIODS = ["24h", "7d", "30d", "90d", "all"] as const;

export type BillingUsagePeriod = (typeof BILLING_USAGE_PERIODS)[number];

export const billingMetricSchema = z.enum(["tokens", "requests", "bytes"]);
export const billingPlanKindSchema = z.enum(["prepaid", "subscription", "hybrid"]);
export const billingCollectionStateSchema = z.enum(["good", "past_due", "suspended"]);
export const billingTopUpStatusSchema = z.enum([
  "pending",
  "checkout_created",
  "failed",
]);

export const billingReadOnlyBlockSchema = z.object({
  charging_enabled: z.boolean(),
  lago_configured: z.boolean(),
  source: z.literal("usage_meter"),
  rates_are_approximate: z.literal(true),
});

export const billingUsageRowSchema = z.object({
  service_slug: z.string().nullable().optional(),
  service_id: z.string().nullable().optional(),
  metric: billingMetricSchema,
  lago_metric_code: z.string(),
  layer: z.string(),
  // Optional so an older backend without the model/agent breakdown still
  // parses; the UI degrades to a service-level row.
  model: z.string().nullable().optional(),
  api_key_id: z.string().nullable().optional(),
  api_key_name: z.string().nullable().optional(),
  quantity: z.number().int(),
  requests: z.number().int(),
  bytes: z.number().int(),
  events: z.number().int().nonnegative(),
  lago_acked: z.boolean(),
  billable: z.boolean().optional().default(true),
  estimated_credits_micros: z.number().int().nullable().optional(),
});

export const billingUsageTotalsSchema = z.object({
  quantity: z.number().int(),
  requests: z.number().int(),
  bytes: z.number().int(),
  events: z.number().int().nonnegative(),
  estimated_credits_micros: z.number().int().nullable().optional(),
});

export const billingUsageResponseSchema = z.object({
  owner_id: z.string().min(1),
  period: z.string().min(1),
  rows: z.array(billingUsageRowSchema),
  totals: billingUsageTotalsSchema,
  billing: billingReadOnlyBlockSchema,
});

export const billingWalletResponseSchema = z.object({
  owner_id: z.string().min(1),
  plan_kind: billingPlanKindSchema,
  collection_state: billingCollectionStateSchema,
  balance_credits: z.number().int(),
  reserved_credits: z.number().int(),
  pending_lago_debits: z.number().int(),
  available_credits: z.number().int(),
  available_with_overdraft_credits: z.number().int(),
  has_payment_instrument: z.boolean(),
  overdraft_cap_credits: z.number().int(),
  suspended: z.boolean(),
  lago_customer_id: z.string().min(1),
  lago_subscription_id: z.string().nullable().optional(),
  lago_wallet_id: z.string().nullable().optional(),
  balance_synced_at: z.string().min(1),
  created_at: z.string().min(1),
  updated_at: z.string().min(1),
  created: z.boolean(),
});

export const provisionBillingWalletRequestSchema = z.object({
  owner_id: z.string().min(1).optional(),
});

export const topUpBillingRequestSchema = z.object({
  amount_credits: z.number().int().positive().max(10_000_000),
  idempotency_key: z.string().trim().min(8).max(128),
  owner_id: z.string().min(1).optional(),
});

export const topUpBillingResponseSchema = z.object({
  owner_id: z.string().min(1),
  amount_credits: z.number().int().positive(),
  idempotency_key: z.string().min(1),
  checkout_url: z.string().url(),
  payment_provider: z.string().nullable().optional(),
  lago_wallet_transaction_id: z.string().nullable().optional(),
  lago_invoice_id: z.string().nullable().optional(),
  status: billingTopUpStatusSchema,
  reused: z.boolean(),
});

export type BillingMetric = z.infer<typeof billingMetricSchema>;
export type BillingPlanKind = z.infer<typeof billingPlanKindSchema>;
export type BillingCollectionState = z.infer<typeof billingCollectionStateSchema>;
export type BillingTopUpStatus = z.infer<typeof billingTopUpStatusSchema>;
export type BillingReadOnlyBlock = z.infer<typeof billingReadOnlyBlockSchema>;
export type BillingUsageRow = z.infer<typeof billingUsageRowSchema>;
export type BillingUsageTotals = z.infer<typeof billingUsageTotalsSchema>;
export type BillingUsageResponse = z.infer<typeof billingUsageResponseSchema>;
export const topUpHistoryEntrySchema = z.object({
  id: z.string().min(1),
  created_at: z.string().min(1),
  amount_credits: z.number().int(),
  status: z.enum(["paid", "pending", "expired", "failed", "voided"]),
  invoice_number: z.string().nullable().optional(),
  lago_invoice_id: z.string().nullable().optional(),
  checkout_url: z.string().nullable().optional(),
  receipt_available: z.boolean(),
});

export const topUpHistoryResponseSchema = z.object({
  owner_id: z.string().min(1),
  topups: z.array(topUpHistoryEntrySchema),
  // Optional with defaults so older backends that omit pagination render
  // a single page instead of failing the parse and blanking the card.
  page: z.number().int().positive().optional().default(1),
  per_page: z.number().int().positive().optional().default(10),
  total: z.number().int().nonnegative().optional().default(0),
});

export const invoiceDownloadResponseSchema = z.object({
  file_url: z.string().min(1),
});

export type TopUpHistoryEntry = z.infer<typeof topUpHistoryEntrySchema>;
export type TopUpHistoryResponse = z.infer<typeof topUpHistoryResponseSchema>;

export type BillingWalletResponse = z.infer<typeof billingWalletResponseSchema>;
export type ProvisionBillingWalletRequest = z.infer<typeof provisionBillingWalletRequestSchema>;
export type TopUpBillingRequest = z.infer<typeof topUpBillingRequestSchema>;
export type TopUpBillingResponse = z.infer<typeof topUpBillingResponseSchema>;
