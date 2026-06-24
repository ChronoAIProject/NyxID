import { z } from "zod";

export const billingMetricSchema = z.enum(["tokens", "requests", "bytes"]);

export const billingReadOnlyBlockSchema = z.object({
  charging_enabled: z.boolean(),
  lago_configured: z.boolean(),
  source: z.string(),
  rates_are_approximate: z.boolean(),
});

export const billingUsageRowSchema = z.object({
  service_slug: z.string().nullable().optional(),
  service_id: z.string().nullable().optional(),
  metric: billingMetricSchema,
  lago_metric_code: z.string(),
  layer: z.string(),
  quantity: z.number().int(),
  requests: z.number().int(),
  bytes: z.number().int(),
  events: z.number().int(),
  lago_acked: z.boolean(),
  estimated_credits_micros: z.number().int().nullable().optional(),
});

export const billingUsageTotalsSchema = z.object({
  quantity: z.number().int(),
  requests: z.number().int(),
  bytes: z.number().int(),
  events: z.number().int(),
  estimated_credits_micros: z.number().int().nullable().optional(),
});

export const billingUsageResponseSchema = z.object({
  owner_id: z.string(),
  period: z.string(),
  rows: z.array(billingUsageRowSchema),
  totals: billingUsageTotalsSchema,
  billing: billingReadOnlyBlockSchema,
});

export const billingInvoiceSummarySchema = z.object({
  id: z.string(),
  status: z.string(),
  amount_credits_micros: z.number().int().nullable().optional(),
  currency: z.string().nullable().optional(),
  hosted_url: z.string().nullable().optional(),
  issued_at: z.string().nullable().optional(),
  due_at: z.string().nullable().optional(),
});

export const billingWalletResponseSchema = z.object({
  owner_id: z.string(),
  charging_enabled: z.boolean(),
  lago_configured: z.boolean(),
  wallet_configured: z.boolean(),
  status: z.string(),
  balance_credits: z.number().int().nullable().optional(),
  reserved_credits: z.number().int().nullable().optional(),
  pending_lago_debits: z.number().int().nullable().optional(),
  available_credits: z.number().int().nullable().optional(),
  source: z.string(),
  invoices: z.array(billingInvoiceSummarySchema),
});

export type BillingMetric = z.infer<typeof billingMetricSchema>;
export type BillingUsageRow = z.infer<typeof billingUsageRowSchema>;
export type BillingUsageTotals = z.infer<typeof billingUsageTotalsSchema>;
export type BillingUsageResponse = z.infer<typeof billingUsageResponseSchema>;
export type BillingWalletResponse = z.infer<typeof billingWalletResponseSchema>;
