import { z } from "zod";

export const PLATFORM_OPERATION_QUERY_KEY = ["admin", "platform-ops"] as const;
export const PLATFORM_OPERATION_DISCOVERY_QUERY_KEY = ["platform-ops"] as const;
const vendorServiceSlugSchema = z
  .string()
  .min(1, "Vendor service slug is required")
  .max(128, "Vendor service slug must be at most 128 characters")
  .regex(/^[a-z0-9-]+$/, "Use only lowercase letters, digits, and hyphens");

const safeIdentifierSchema = (label: string) =>
  z
    .string()
    .min(1, `${label} is required`)
    .max(128, `${label} must be at most 128 characters`)
    .regex(
      /^[A-Za-z0-9._-]+$/,
      `${label} may only use letters, digits, periods, hyphens, and underscores`,
    );

const e164Schema = z
  .string()
  .regex(/^\+[1-9][0-9]{0,14}$/, "Must be a valid E.164 number or prefix");

const uniqueStrings = <T extends z.ZodType<string>>(item: T, max: number) =>
  z
    .array(item)
    .max(max, `At most ${String(max)} values are allowed`)
    .refine(
      (values) => new Set(values).size === values.length,
      "Duplicate values are not allowed",
    );

/// Mirrors `BillingMetric` in backend/src/models/service_billing.rs. The
/// admin surface previously accepted only "requests", which rejected the
/// default `speak` (characters) and `call_and_say` (seconds) rows.
export const billingMetricSchema = z.enum([
  "tokens",
  "requests",
  "bytes",
  "characters",
  "seconds",
  "input_tokens",
  "output_tokens",
]);

export const pricingSyncStatusSchema = z.enum(["pending", "synced", "failed"]);

/// `display` is rendered by the backend so a per-second or per-character
/// price cannot be relabelled as a per-call price by a client.
const pricingResponseSchema = z
  .object({
    billable: z.boolean(),
    metric: billingMetricSchema,
    price_per_unit: z.string().min(1),
    secondary: z
      .object({
        metric: billingMetricSchema,
        price_per_unit: z.string().min(1),
      })
      .strict()
      .nullable(),
    base_fee_per_call: z.string().min(1).nullable(),
    display: z.string().min(1),
  })
  .strict();

const adminPricingResponseSchema = z
  .object({
    billable: z.boolean(),
    metric: billingMetricSchema,
    price_per_unit: z.string().min(1),
    secondary: z
      .object({
        metric: billingMetricSchema,
        price_per_unit: z.string().min(1),
        lago_metric_code: z.string(),
      })
      .strict()
      .nullable(),
    base_fee_per_call: z.string().min(1).nullable(),
    display: z.string().min(1),
    // Empty until the operation price has been synchronized to Lago.
    lago_metric_code: z.string(),
    sync_status: pricingSyncStatusSchema,
    sync_error: z.string().nullable(),
  })
  .strict();

const operationMetadataSchema = {
  operation_id: z.string().uuid().optional(),
  enabled: z.boolean(),
  vendor_service_slug: vendorServiceSlugSchema,
  updated_at: z.string().nullable(),
  updated_by: z.string().nullable(),
  vendor_service_id: z.string().min(1).nullable(),
  pricing: adminPricingResponseSchema,
};

export const speakConfigResponseSchema = z
  .object({
    type: z.literal("speak"),
    allowed_voice_ids: uniqueStrings(safeIdentifierSchema("Voice ID"), 100),
    max_chars: z
      .number()
      .int("Maximum characters must be an integer")
      .min(1, "Maximum characters must be at least 1")
      .max(5_000, "Maximum characters cannot exceed 5000"),
    model_id: safeIdentifierSchema("Model ID"),
    // Speak is priced per character, so a per-call count is a coarse bound.
    // It is still the difference between a looping agent spending a bounded
    // amount and spending until the wallet stops it.
    max_calls_per_user_per_day: z
      .number()
      .int("Daily call limit must be an integer")
      .min(1, "Daily call limit must be at least 1")
      .max(4_294_967_295, "Daily call limit is too large"),
  })
  .strict();

export const speakConfigSchema = speakConfigResponseSchema.extend({
  allowed_voice_ids: uniqueStrings(safeIdentifierSchema("Voice ID"), 100).min(
    1,
    "Add at least one allowed voice ID",
  ),
});

export const callAndSayConfigResponseSchema = z
  .object({
    type: z.literal("call_and_say"),
    allowed_destination_prefixes: uniqueStrings(e164Schema, 100),
    max_message_chars: z
      .number()
      .int("Maximum message characters must be an integer")
      .min(1, "Maximum message characters must be at least 1")
      .max(1_000, "Maximum message characters cannot exceed 1000"),
    // Bounds a per-second-billed call. Omitting it from an update lets the
    // backend serde default silently reset the cap, so it must round-trip.
    max_duration_seconds: z
      .number()
      .int("Maximum call duration must be an integer")
      .min(1, "Maximum call duration must be at least 1 second")
      .max(3_600, "Maximum call duration cannot exceed 3600 seconds"),
    voice: safeIdentifierSchema("Voice"),
    max_calls_per_user_per_day: z
      .number()
      .int("Daily call limit must be an integer")
      .min(1, "Daily call limit must be at least 1")
      .max(4_294_967_295, "Daily call limit is too large"),
    account_sid: z.union([
      z.literal(""),
      z
        .string()
        .regex(
          /^AC[0-9A-Fa-f]{32}$/,
          "Must be a Twilio Account SID beginning with AC",
        ),
    ]),
    call_from: z.union([z.literal(""), e164Schema]),
  })
  .strict();

export const callAndSayConfigSchema = callAndSayConfigResponseSchema.extend({
  account_sid: z
    .string()
    .regex(
      /^AC[0-9A-Fa-f]{32}$/,
      "Must be a Twilio Account SID beginning with AC",
    ),
  call_from: e164Schema,
});

export const flightSearchConfigSchema = z
  .object({
    type: z.literal("flight_search"),
    max_offers_cap: z
      .number()
      .int("Maximum offers must be an integer")
      .min(1, "Maximum offers must be at least 1")
      .max(50, "Maximum offers cannot exceed 50"),
    max_searches_per_user_per_day: z
      .number()
      .int("Daily search limit must be an integer")
      .min(1, "Daily search limit must be at least 1")
      .max(4_294_967_295, "Daily search limit is too large"),
  })
  .strict();

export const speakOperationSchema = z
  .object({
    op: z.literal("speak"),
    ...operationMetadataSchema,
    config: speakConfigResponseSchema,
  })
  .strict();

export const callAndSayOperationSchema = z
  .object({
    op: z.literal("call_and_say"),
    ...operationMetadataSchema,
    config: callAndSayConfigResponseSchema,
  })
  .strict();

export const flightSearchOperationSchema = z
  .object({
    op: z.literal("flight_search"),
    ...operationMetadataSchema,
    config: flightSearchConfigSchema,
  })
  .strict();

export const platformOperationSchema = z.discriminatedUnion("op", [
  speakOperationSchema,
  callAndSayOperationSchema,
  flightSearchOperationSchema,
]);

export const legacyPlatformOperationListSchema = z
  .object({
    operations: z.array(platformOperationSchema),
  })
  .strict();

const adminEndpointKindSchema = z
  .object({
    type: z.literal("endpoint"),
    method: z.string().min(1),
    path_template: z.string().min(1),
    name: z.string().min(1).max(160),
    description: z.string().max(4_096).nullable(),
  })
  .strict();

const adminConstrainedConfigSchema = z.discriminatedUnion("type", [
  z
    .object({
      type: z.literal("speak"),
      allowed_voice_ids: z.array(z.string()),
      model_id: z.string().min(1),
      max_calls_per_user_per_day: z.number().int().positive(),
    })
    .strict(),
  z
    .object({
      type: z.literal("call_and_say"),
      allowed_destination_prefixes: z.array(z.string()),
      voice: z.string().min(1),
      account_sid: z.string(),
      call_from: z.string(),
    })
    .strict(),
  z.object({ type: z.literal("flight_search") }).strict(),
]);

const adminConstrainedKindSchema = z
  .object({
    type: z.literal("constrained"),
    op: z.enum(["speak", "call_and_say", "flight_search"]),
    config: adminConstrainedConfigSchema,
  })
  .strict();

const adminPerRequestCapsSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("endpoint") }).strict(),
  z
    .object({
      type: z.literal("speak"),
      max_chars: z.number().int().positive(),
    })
    .strict(),
  z
    .object({
      type: z.literal("call_and_say"),
      max_message_chars: z.number().int().positive(),
      max_duration_seconds: z.number().int().positive(),
    })
    .strict(),
  z
    .object({
      type: z.literal("flight_search"),
      max_offers: z.number().int().positive(),
    })
    .strict(),
]);

export const adminPlatformOperationSchema = z
  .object({
    operation_id: z.string().uuid(),
    catalog_service_id: z.string().min(1),
    provider_slug: vendorServiceSlugSchema.nullable(),
    provider_name: z.string().min(1).nullable(),
    operation_name: z.string().min(1),
    enabled: z.boolean(),
    kind: z.union([adminEndpointKindSchema, adminConstrainedKindSchema]),
    limits: z
      .object({
        per_request: adminPerRequestCapsSchema,
        per_user_per_day: z.number().int().positive().nullable(),
      })
      .strict(),
    pricing: adminPricingResponseSchema,
    created_at: z.string().min(1),
    created_by: z.string().min(1),
    updated_at: z.string().min(1),
    updated_by: z.string().min(1),
  })
  .strict();

export const platformOperationListSchema = z
  .object({
    operations: z.array(adminPlatformOperationSchema),
  })
  .strict();

const normalizedCreditAmountSchema = z
  .string()
  .trim()
  .regex(
    /^\d+(?:\.\d{1,6})?$/,
    "Use a non-negative credit amount with at most 6 decimal places",
  );

const updatePlatformOperationBillingComponentSchema = z
  .object({
    metric: billingMetricSchema,
    price_per_unit: normalizedCreditAmountSchema.refine(
      (value) => Number(value) > 0,
      "Secondary component price must be greater than zero",
    ),
  })
  .strict();

export const updatePlatformOperationBillingSchema = z
  .object({
    metric: billingMetricSchema,
    price_per_unit: normalizedCreditAmountSchema,
    secondary: updatePlatformOperationBillingComponentSchema.nullable(),
    base_fee_per_call: normalizedCreditAmountSchema.nullable(),
  })
  .strict()
  .superRefine((billing, context) => {
    if (billing.secondary?.metric === billing.metric) {
      context.addIssue({
        code: "custom",
        message: "Billing components must use different metrics",
        path: ["secondary", "metric"],
      });
    }
  });

const updateEndpointKindSchema = z
  .object({
    kind: z.literal("endpoint"),
    method: z.string().trim().min(1).max(16),
    path_template: z.string().trim().startsWith("/").max(2_048),
    name: z.string().trim().min(1).max(160),
    description: z.string().trim().max(4_096).nullable(),
  })
  .strict();

const updateConstrainedKindSchema = z
  .object({
    kind: z.literal("constrained"),
    op: z.enum(["speak", "call_and_say", "flight_search"]),
    config: adminConstrainedConfigSchema,
  })
  .strict();

export const updatePlatformOperationSchema = z
  .object({
    enabled: z.boolean(),
    kind: z.union([updateEndpointKindSchema, updateConstrainedKindSchema]),
    limits: z
      .object({
        per_request: adminPerRequestCapsSchema,
        per_user_per_day: z.number().int().positive(),
      })
      .strict(),
    billing: updatePlatformOperationBillingSchema,
  })
  .strict();

export const adminPlatformCredentialStatusSchema = z
  .object({
    configured: z.boolean(),
    id: z.string().uuid().nullable(),
    auth_method: z.string().min(1).nullable(),
    auth_key_name: z.string().min(1).nullable(),
    created_at: z.string().min(1).nullable(),
    updated_at: z.string().min(1).nullable(),
  })
  .strict();

export const adminPlatformProviderSchema = z
  .object({
    catalog_service_id: z.string().min(1),
    catalog_service_slug: vendorServiceSlugSchema,
    catalog_service_name: z.string().min(1),
    catalog_service_active: z.boolean(),
    eligible: z.boolean(),
    eligibility_reason: z.string().min(1).nullable(),
    promoted: z.boolean(),
    promoted_at: z.string().min(1).nullable(),
    promoted_by: z.string().min(1).nullable(),
    vendor_terms_accepted_at: z.string().min(1).nullable(),
    vendor_terms_accepted_by: z.string().min(1).nullable(),
    credential: adminPlatformCredentialStatusSchema,
    enabled_operation_count: z.number().int().nonnegative(),
  })
  .strict();

export const adminPlatformProviderListSchema = z
  .object({ providers: z.array(adminPlatformProviderSchema) })
  .strict();

export const platformCredentialWriteSchema = z
  .object({
    credential: z
      .string()
      .min(1, "Credential is required")
      .max(65_536, "Credential is too large"),
  })
  .strict();

const updateMetadataSchema = {
  enabled: z.boolean(),
  vendor_service_slug: vendorServiceSlugSchema,
};

export const speakUpdateSchema = z
  .object({
    ...updateMetadataSchema,
    config: speakConfigSchema,
  })
  .strict();

export const callAndSayUpdateSchema = z
  .object({
    ...updateMetadataSchema,
    config: callAndSayConfigSchema,
  })
  .strict();

export const flightSearchUpdateSchema = z
  .object({
    ...updateMetadataSchema,
    config: flightSearchConfigSchema,
  })
  .strict();

export const platformOperationDiscoverySchema = z
  .object({
    op: z.enum(["speak", "call_and_say", "flight_search"]),
    display_name: z.string().min(1),
    description: z.string().min(1),
    vendor: z.string().min(1),
    catalog_service_slug: z.string().min(1),
    credential_source: z.enum(["platform", "own_connection", "unavailable"]),
    credential_intent: z.enum(["auto", "own_only", "platform_only"]),
    availability_reason: z
      .enum(["owner_opt_in_required", "own_connection_disabled"])
      .nullable(),
    fallback_reason: z
      .enum([
        "own_credential_absent",
        "own_credential_unusable",
        "explicit_platform_only",
      ])
      .nullable(),
    own_connection: z
      .object({
        user_service_id: z.string().min(1),
        slug: z.string().min(1),
        label: z.string().min(1),
        is_active: z.boolean(),
        usable: z.boolean(),
        reason: z
          .enum([
            "disabled",
            "node_routed",
            "unusable",
            "approval_required",
            // The connection is fine; this API key is not scoped to it. Calls
            // fail closed rather than falling back to platform credits.
            "out_of_scope",
          ])
          .nullable(),
      })
      .strict()
      .nullable(),
    pricing: pricingResponseSchema,
    mcp_tool: z.string().min(1),
  })
  .strict();

export const platformOperationDiscoveryListSchema = z
  .object({
    operations: z.array(platformOperationDiscoverySchema),
  })
  .strict();

export type BillingMetric = z.infer<typeof billingMetricSchema>;
export type PricingSyncStatus = z.infer<typeof pricingSyncStatusSchema>;
export type PlatformOperationPricing = z.infer<
  typeof adminPricingResponseSchema
>;
export type PlatformOperationDiscoveryPricing = z.infer<
  typeof pricingResponseSchema
>;
export type PlatformOperation = z.infer<typeof platformOperationSchema>;
export type PlatformOperationList = z.infer<
  typeof legacyPlatformOperationListSchema
>;
export type AdminPlatformOperation = z.infer<
  typeof adminPlatformOperationSchema
>;
export type AdminPlatformOperationList = z.infer<
  typeof platformOperationListSchema
>;
export type UpdateAdminPlatformOperation = z.infer<
  typeof updatePlatformOperationSchema
>;
export type AdminPlatformProvider = z.infer<typeof adminPlatformProviderSchema>;
export type AdminPlatformProviderList = z.infer<
  typeof adminPlatformProviderListSchema
>;
export type PlatformCredentialWrite = z.infer<
  typeof platformCredentialWriteSchema
>;
export type SpeakOperation = z.infer<typeof speakOperationSchema>;
export type CallAndSayOperation = z.infer<typeof callAndSayOperationSchema>;
export type FlightSearchOperation = z.infer<typeof flightSearchOperationSchema>;
export type SpeakUpdate = z.infer<typeof speakUpdateSchema>;
export type CallAndSayUpdate = z.infer<typeof callAndSayUpdateSchema>;
export type FlightSearchUpdate = z.infer<typeof flightSearchUpdateSchema>;
export type PlatformOperationDiscovery = z.infer<
  typeof platformOperationDiscoverySchema
>;
export type PlatformOperationDiscoveryList = z.infer<
  typeof platformOperationDiscoveryListSchema
>;

export type UpdatePlatformOperationVariables =
  | { readonly op: "speak"; readonly data: SpeakUpdate }
  | { readonly op: "call_and_say"; readonly data: CallAndSayUpdate }
  | { readonly op: "flight_search"; readonly data: FlightSearchUpdate };
