import { z } from "zod";
export const billingSettingsSchema = z.object({ default_lane: z.string().nullable(), default_overdraft_cap_credits: z.number().int().nonnegative().nullable() });
export const billingConfigSchema = z.object({ billing_enabled:z.boolean(), lago_configured:z.boolean(), resale_enabled:z.boolean(), global:billingSettingsSchema.nullable(), precedence:z.array(z.string()) });
export const billingOverviewSchema = z.array(z.object({ id:z.string(), slug:z.string(), name:z.string(), has_billing:z.boolean(), effective_source:z.string() }));
export const billingRateSchema = z.object({ id:z.string(), metric:z.string(), model:z.string().nullable(), credits_per_unit_micros:z.number(), synced_at:z.string(), retired_at:z.string().nullable(), stale:z.boolean() });
export type BillingSettings = z.infer<typeof billingSettingsSchema>;
