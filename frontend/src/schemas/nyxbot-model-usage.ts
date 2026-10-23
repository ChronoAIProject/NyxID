import { z } from "zod";
import {
  usageIdentitySchema,
  usageServiceIdentitySchema,
  usageStatsSchema,
} from "./admin-usage";
export const modelStatsSchema = usageStatsSchema
  .omit({
    quantities: true,
    unique_users: true,
    unique_services: true,
    gross_cost_micros: true,
    wallet_cost_micros: true,
    grant_cost_micros: true,
    allowance_cost_micros: true,
  })
  .required();
const serviceUsageSchema = z.object({
  service: usageServiceIdentitySchema.nullable(),
  is_other: z.boolean(),
  groups: z.number().int().nonnegative(),
  usage: modelStatsSchema,
});
const modelUsageSchema = z.object({
  model: z.string().nullable(),
  is_other: z.boolean(),
  groups: z.number().int().nonnegative(),
  usage: modelStatsSchema,
});
export const nyxbotModelUsageSchema = z.object({
  window: z.object({
    from: z.iso.datetime({ offset: true }),
    to: z.iso.datetime({ offset: true }),
    period: z.string(),
  }),
  freshness: z.object({
    rolled_up_through: z.iso.datetime({ offset: true }),
    tail_rows: z.number().int().nonnegative(),
    validated: z.boolean(),
  }),
  totals: modelStatsSchema,
  models: z.array(modelUsageSchema),
  daily: z.array(
    modelUsageSchema.extend({ day: z.iso.datetime({ offset: true }) }),
  ),
  accounts: z.array(
    z.object({
      account: usageIdentitySchema,
      usage: modelStatsSchema,
      models: z.array(modelUsageSchema),
      other_model_count: z.number().int().nonnegative(),
    }),
  ),
  other_accounts: modelUsageSchema.nullable(),
  // Absent from servers that predate the service breakdown.
  services: z.array(serviceUsageSchema).default([]),
  excluded: z.array(
    z.object({ population: z.string(), usage: modelStatsSchema }),
  ),
});
export type NyxbotModelUsage = z.infer<typeof nyxbotModelUsageSchema>;
export type NyxbotUsageModel = z.infer<typeof modelUsageSchema>;
export type NyxbotUsageService = z.infer<typeof serviceUsageSchema>;

export type NyxbotModelStats = z.infer<typeof modelStatsSchema>;
