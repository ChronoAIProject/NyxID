import { z } from "zod";

/** Where an effective value comes from; mirrors `BillingConfigSource`. */
export const billingSourceSchema = z.enum([
  "env",
  "global",
  "service",
  "legacy",
  "free",
]);
export type BillingSource = z.infer<typeof billingSourceSchema>;

const syncStatusSchema = z.enum(["pending", "synced", "failed"]);

export const billingConfigValueSchema = z.object({
  key: z.string(),
  value: z.string().nullable(),
  source: billingSourceSchema,
  secret_set: z.boolean().nullable(),
});
export type BillingConfigValue = z.infer<typeof billingConfigValueSchema>;

export const billingConfigSchema = z.object({
  billing_enabled: z.boolean(),
  lago_configured: z.boolean(),
  resale_enabled: z.boolean(),
  default_overdraft_cap_credits: z.object({
    effective: z.number().int(),
    source: billingSourceSchema,
    global: z.number().int().nullable(),
    env: z.number().int(),
  }),
  max_default_overdraft_cap_credits: z.number().int().positive(),
  environment: z.array(billingConfigValueSchema),
});
export type BillingConfig = z.infer<typeof billingConfigSchema>;

/** Per-field patch: a number sets the global value, `null` clears it to the env default. */
export type BillingConfigPatch = {
  readonly default_overdraft_cap_credits: number | null;
};

/** Settings form: an empty cap means "use the environment default". */
export function billingSettingsFormSchema(max: number) {
  return z.object({
    default_overdraft_cap_credits: z
      .string()
      .trim()
      .refine(
        (value) =>
          value === "" ||
          (/^\d+$/.test(value) && Number(value) >= 0 && Number(value) <= max),
        `Enter a whole number of credits between 0 and ${max}, or leave empty to use the environment default`,
      ),
  });
}
export type BillingSettingsForm = z.infer<
  ReturnType<typeof billingSettingsFormSchema>
>;

export function billingSettingsPatch(
  form: BillingSettingsForm,
): BillingConfigPatch {
  const value = form.default_overdraft_cap_credits.trim();
  return { default_overdraft_cap_credits: value === "" ? null : Number(value) };
}

export const effectivePriceSchema = z.object({
  layer: z.enum(["platform", "resale"]),
  metric: z.string(),
  operation: z.string().nullable(),
  lago_metric_code: z.string(),
  credits_per_unit: z.string().nullable(),
  source: billingSourceSchema,
  origin: z.enum([
    "lane_primary",
    "lane_component",
    "operation",
    "service_price",
    "legacy_plan",
    "resale_plan",
  ]),
  sync_status: syncStatusSchema.nullable(),
  sync_error: z.string().nullable(),
  cached_rate: z
    .object({
      credits_per_unit: z.string(),
      synced_at: z.string(),
      stale: z.boolean(),
      retired: z.boolean(),
    })
    .nullable(),
});
export type EffectivePrice = z.infer<typeof effectivePriceSchema>;

export const credentialClassSchema = z.enum([
  "nyxid_managed_master",
  "nyxid_platform_oauth_app",
  "user_owned",
  "agent_override_user_owned",
  "node_managed",
  "no_auth",
]);
export type CredentialClass = z.infer<typeof credentialClassSchema>;

export const effectiveReasonSchema = z.enum([
  "billing_disabled",
  "nyxid_credentials_only",
  "lane_synced",
  "lane_unsynced_legacy",
  "lane_unsynced_free",
  "lane_not_configured",
  "no_auth",
  "legacy_platform_billable",
  "not_billable",
]);
export type EffectiveReason = z.infer<typeof effectiveReasonSchema>;

export const effectiveClassSchema = z.object({
  credential_classes: z.array(credentialClassSchema).min(1),
  lane: z.enum(["byok", "platform_key"]).nullable(),
  platform_charged: z.boolean(),
  source: billingSourceSchema,
  reason: effectiveReasonSchema,
  prices: z.array(effectivePriceSchema),
  pending: z.array(effectivePriceSchema),
});
export type EffectiveClass = z.infer<typeof effectiveClassSchema>;

export const billingServiceSchema = z.object({
  id: z.string(),
  slug: z.string(),
  name: z.string(),
  is_active: z.boolean(),
  effective_metric: z.string(),
  allowance_metrics: z.array(z.string()),
  effective: z.object({
    fallback_metric: z.string(),
    classes: z.array(effectiveClassSchema),
    resale: z.object({
      configured: z.boolean(),
      applied: z.boolean(),
      source: billingSourceSchema,
    }),
  }),
});
export type BillingService = z.infer<typeof billingServiceSchema>;
export const billingOverviewSchema = z.array(billingServiceSchema);

export const resyncResponseSchema = z.object({
  synced: z.boolean(),
  service: billingServiceSchema,
});

export const billingRateSchema = z.object({
  id: z.string(),
  lago_metric_code: z.string(),
  model: z.string().nullable(),
  credits_per_unit: z.string(),
  synced_at: z.string(),
  stale_after: z.string(),
  stale: z.boolean(),
  retired_at: z.string().nullable(),
});
export type BillingRate = z.infer<typeof billingRateSchema>;
