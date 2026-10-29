import { z } from "zod";
import { creditUnits, legacyCredits, sumCredits } from "./credits";

export const billingPeriodSchema = z.enum(["24h", "7d", "30d"]);
export type BillingPeriod = z.infer<typeof billingPeriodSchema>;
const count = z.number().int().safe().nonnegative();
// Unselected legacy mirrors may exceed JS precision. Only a selected fallback
// must be safe; exact strings (including null) remain authoritative.
const legacy = z.number().refine(Number.isInteger).nullable().optional();
const instant = z.string().datetime({ offset: true });
export const creditsSchema = z.string().max(80).refine((value) => {
  try { creditUnits(value); return true; } catch { return false; }
}, "Invalid exact credit amount");
const money = creditsSchema.nullable().optional();

function exact(value: string | null | undefined, fallback: number | null | undefined, scale: "whole" | "micros" = "micros"): string | null {
  // A present exact null is authoritative; legacy mirrors must not overwrite it.
  return value !== undefined ? value : fallback == null ? null : legacyCredits(fallback, scale);
}

export const walletSchema = z.object({
  balance: creditsSchema, reserved: creditsSchema, pending_debits: creditsSchema,
  pending_expiry: creditsSchema, available: creditsSchema, overdraft_cap: creditsSchema,
  // Earlier development responses omitted the unit; this endpoint always uses credits.
  unit: z.literal("credits").default("credits"),
}).transform((row) => ({
  balance: row.balance, reserved: row.reserved, pendingDebits: row.pending_debits,
  pendingExpiry: row.pending_expiry, available: row.available, overdraftCap: row.overdraft_cap,
  unit: row.unit,
}));

const fundingFields = {
  estimated_credits: money, wallet_credits: money, grant_credits: money, allowance_credits: money,
  estimated_credits_micros: legacy, wallet_credits_micros: legacy,
  grant_credits_micros: legacy, allowance_credits_micros: legacy,
};
const fundingSchema = z.object(fundingFields).transform((row) => ({
  cost: exact(row.estimated_credits, row.estimated_credits_micros),
  wallet: exact(row.wallet_credits, row.wallet_credits_micros),
  grants: exact(row.grant_credits, row.grant_credits_micros),
  allowances: exact(row.allowance_credits, row.allowance_credits_micros),
}));
const usageRowSchema = z.object({
  ...fundingFields,
  service_slug: z.string().nullish(), service_id: z.string().nullish(),
  metric: z.string().min(1), layer: z.string(), model: z.string().nullish(),
  quantity: z.number().int().safe(), billable: z.boolean().default(true),
}).transform((row) => ({
  service: row.service_slug ?? "Other service", serviceId: row.service_id,
  metric: row.metric, layer: row.layer, model: row.model, quantity: row.quantity,
  billable: row.billable, ...fundingSchema.parse(row),
}));
export const usageSchema = z.object({
  owner_id: z.string().min(1), period: billingPeriodSchema, rows: z.array(usageRowSchema),
  totals: fundingSchema,
  billing: z.object({ charging_enabled: z.boolean(), rates_are_approximate: z.boolean() }),
});

const scopeSchema = z.object({ all_services: z.boolean(), service_ids: z.array(z.string()), service_slugs: z.array(z.string()) });
const serviceLimitSchema = z.object({
  key: z.string(), label: z.string().nullish(), service_ids: z.array(z.string()), service_slugs: z.array(z.string()),
  limit: creditsSchema, consumed: creditsSchema, reserved: creditsSchema, held: creditsSchema.optional(),
});
const grantSchema = z.object({
  id: z.string().min(1), amount: money, remaining: money, reserved: money,
  held: creditsSchema.optional(), revoke_requested_at: instant.nullish(),
  amount_micros: legacy, remaining_micros: legacy, reserved_micros: legacy,
  activation_state: z.enum(["active", "pending_activation"]),
  status: z.enum(["active", "consumed", "expired", "revoked"]),
  expires_at: instant.nullish(), reason: z.string().nullish(), scope: scopeSchema,
  service_limits: z.array(serviceLimitSchema).default([]),
}).transform((row) => ({
  ...row, amount: exact(row.amount, row.amount_micros), remaining: exact(row.remaining, row.remaining_micros),
  reserved: exact(row.reserved, row.reserved_micros),
}));
export const grantsSchema = z.object({ grants: z.array(grantSchema) });
export const allowancesSchema = z.object({ allowances: z.array(z.object({
  allowance: z.object({
    id: z.string().min(1), service_slug: z.string().min(1), metric: z.string().min(1), quantity: count,
    recurrence: z.enum(["one_time", "daily", "weekly", "monthly"]), is_active: z.boolean(),
  }),
  period_start: instant, period_end: instant.nullish(),
  consumed_quantity: count, reserved_quantity: count, remaining_quantity: count,
})) });
export const billingOrganizationsSchema = z.object({ orgs: z.array(z.object({
  id: z.string().uuid(), slug: z.string().min(1), display_name: z.string().nullish(), your_role: z.string(),
})) }).transform(({ orgs }) => orgs.filter((org) => org.your_role === "admin" || org.your_role === "member"));

export type BillingWallet = z.infer<typeof walletSchema>;
export type BillingUsage = z.infer<typeof usageSchema>;
export type BillingGrant = z.infer<typeof grantSchema>;
export type BillingAllowance = z.infer<typeof allowancesSchema>["allowances"][number];

export function serviceUsage(rows: BillingUsage["rows"]) {
  const groups = new Map<string, typeof rows>();
  for (const row of rows) {
    const key = row.serviceId ?? row.service;
    const group = groups.get(key);
    if (group) group.push(row);
    else groups.set(key, [row]);
  }
  return [...groups].map(([key, entries]) => ({
    key, name: entries[0]!.service,
    cost: sumCredits(entries.map((row) => row.billable ? row.cost : "0")),
    wallet: sumCredits(entries.map((row) => row.billable ? row.wallet : "0")),
    grants: sumCredits(entries.map((row) => row.billable ? row.grants : "0")),
    rows: entries,
    meteringOnly: entries.every((row) => !row.billable),
  })).sort((a, b) => a.name.localeCompare(b.name));
}
