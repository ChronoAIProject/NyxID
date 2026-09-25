import { z } from "zod";
import { BILLING_METRICS } from "./billing-metrics";
import { usageStatsSchema } from "./admin-usage";

export const ANALYTICS_MEASURES = [
  "cost",
  "requests",
  "events",
  "exact_cost_events",
  "legacy_cost_events",
  "unknown_cost_events",
  "total_tokens",
  "prompt_tokens",
  "completion_tokens",
  "cached_tokens",
  "cache_creation_tokens",
  "quantity",
  "wallet_cost",
  "grant_cost",
  "allowance_cost",
] as const;
export const CHART_TYPES = ["line", "bar", "pie", "combo"] as const;
export const LAYOUTS = ["operations", "overview", "explorer"] as const;
export const ANALYTICS_INTERVALS = [
  "auto",
  "hour",
  "day",
  "week",
  "month",
] as const;
const filterStrings = z
  .array(
    z
      .string()
      .trim()
      .min(1)
      .max(256)
      .refine((s) => !s.includes(",")),
  )
  .max(20);
export const analyticsFiltersSchema = z.object({
  period: z.enum(["24h", "7d", "30d", "custom"]),
  from: z.string().nullable(),
  to: z.string().nullable(),
  services: filterStrings,
  actors: z.array(z.uuid()).max(20),
  owners: z.array(z.uuid()).max(20),
});
const title = z
  .string()
  .refine((s) => s.trim().length > 0, "Enter a title.")
  .refine(
    (s) => new TextEncoder().encode(s).length <= 100,
    "Use at most 100 bytes.",
  );
export const analyticsPanelSchema = z
  .object({
    id: z.uuid(),
    title,
    chart: z.enum(CHART_TYPES),
    measure: z.enum(ANALYTICS_MEASURES),
    metric: z.enum(BILLING_METRICS),
    breakdown: z.enum(["service", "user", "owner", "credential_class"]),
    top: z.union([z.literal(0), z.literal(5), z.literal(10)]),
    wide: z.boolean(),
    span: z.union([z.literal(1), z.literal(2), z.literal(3)]).optional(),
    height: z.enum(["compact", "standard", "tall"]).optional(),
    interval: z.enum(ANALYTICS_INTERVALS).optional(),
  })
  .refine(
    (panel) => panel.chart !== "combo" || panel.measure !== "requests",
    "Use a single axis for requests.",
  );
export const analyticsViewSchema = z.object({
  id: z.uuid(),
  name: title,
  layout: z.enum(LAYOUTS),
  filters: analyticsFiltersSchema,
  panels: z.array(analyticsPanelSchema).min(1),
});
export const workspaceConfigSchema = z
  .object({
    version: z.literal(1),
    draft: analyticsViewSchema,
    saved_views: z.array(analyticsViewSchema).max(20),
  })
  .refine(
    (config) =>
      new TextEncoder().encode(JSON.stringify(config)).length <= 1_048_576,
    "Workspace settings must fit within 1 MiB. Remove unused saved views or panels to save.",
  );
export const workspaceResponseSchema = z.object({
  revision: z.number().int().nonnegative(),
  config: workspaceConfigSchema.nullable(),
});
const amount = z.number().int().nonnegative().nullable();
const analyticsPointSchema = z.object({
  bucket: z.iso.datetime({ offset: true }),
  value: amount,
  requests: z.number().nonnegative(),
  unknown_cost_events: z.number().nonnegative(),
});
export const analyticsResponseSchema = z.object({
  window: z.object({
    from: z.iso.datetime({ offset: true }),
    to: z.iso.datetime({ offset: true }),
    period: z.string(),
  }),
  freshness: z.object({
    rolled_up_through: z.iso.datetime({ offset: true }),
    tail_rows: z.number(),
    validated: z.boolean(),
  }),
  granularity: z.enum(["hour", "day", "week", "month"]),
  unit: z.string(),
  total: amount,
  totals: usageStatsSchema,
  points: z.array(analyticsPointSchema),
  series: z
    .array(
      z.object({
        label: z.string(),
        is_other: z.boolean(),
        points: z.array(analyticsPointSchema),
      }),
    )
    .max(11),
  slices: z.array(
    z.object({
      id: z.string().nullable(),
      label: z.string(),
      value: amount,
      unknown_cost_events: z.number().nonnegative(),
      is_other: z.boolean(),
    }),
  ),
});
export type AnalyticsFilters = z.infer<typeof analyticsFiltersSchema>;
export type AnalyticsPanel = z.infer<typeof analyticsPanelSchema>;
export type AnalyticsView = z.infer<typeof analyticsViewSchema>;
export type WorkspaceConfig = z.infer<typeof workspaceConfigSchema>;
export type WorkspaceResponse = z.infer<typeof workspaceResponseSchema>;
export type AnalyticsResult = z.infer<typeof analyticsResponseSchema>;
export type AnalyticsLayout = AnalyticsView["layout"];
