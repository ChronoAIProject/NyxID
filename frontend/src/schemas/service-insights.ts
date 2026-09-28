import { z } from "zod";

export const serviceBillingExplanationSchema = z.object({
  status: z.enum(["resolved", "conditional", "restricted", "unavailable"]),
  credential_class: z.string().nullable(),
  credential_label: z.string(),
  account: z
    .object({
      id: z.string(),
      kind: z.enum(["personal", "organization"]),
      name: z.string(),
    })
    .nullable(),
  charge_status: z.enum([
    "usage_based",
    "not_charged",
    "conditional",
    "restricted",
    "unavailable",
  ]),
  rates: z.array(
    z.object({
      layer: z.string(),
      metric: z.string(),
      credits_per_unit: z.string().nullable(),
      currency: z.string(),
      source: z.string(),
    }),
  ),
  provider_billing: z.enum([
    "separate_provider_account",
    "nyxid_credential",
    "no_credential",
    "unknown",
  ]),
  context: z.string(),
  notes: z.array(z.string()),
});

export const serviceCallerSchema = z.object({
  id: z.string().nullable(),
  kind: z.string(),
  name: z.string(),
  app_id: z.string().nullable(),
  app_name: z.string().nullable(),
});

export const connectionActivitySchema = z.object({
  access: z.object({
    visibility: z.string(),
    keys: z.array(
      z.object({
        id: z.string(),
        name: z.string(),
        platform: z.string().nullable(),
        owner_id: z.string(),
        permission: z.string(),
        credential_override: z.boolean(),
      }),
    ),
    truncated: z.boolean(),
  }),
  activity: z.object({
    visibility: z.string(),
    period_days: z.number(),
    tracking: z.string(),
    request_count: z.number(),
    requests: z.array(
      z.object({
        id: z.string(),
        execution_id: z.string().nullable(),
        caller: serviceCallerSchema,
        occurred_at: z.string(),
        outcome: z.string(),
        response_status: z.number().nullable(),
      }),
    ),
    truncated: z.boolean(),
  }),
});

export const serviceInsightSchema = z.object({
  service_id: z.string(),
  billing: serviceBillingExplanationSchema.nullable(),
  usage: connectionActivitySchema.nullable(),
});
export const serviceInsightsResponseSchema = z.object({
  connections: z.array(serviceInsightSchema),
});
export type ServiceInsight = z.infer<typeof serviceInsightSchema>;
export type ConnectionActivity = z.infer<typeof connectionActivitySchema>;
export type ServiceBillingExplanation = z.infer<
  typeof serviceBillingExplanationSchema
>;
export type ServiceCaller = z.infer<typeof serviceCallerSchema>;
