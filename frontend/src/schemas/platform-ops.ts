import { z } from "zod";

export const PLATFORM_OPERATION_QUERY_KEY = [
  "admin",
  "platform-ops",
] as const;
export const PLATFORM_OPERATION_DISCOVERY_QUERY_KEY = [
  "platform-ops",
] as const;
const vendorServiceSlugSchema = z
  .string()
  .min(1, "Vendor service slug is required")
  .max(128, "Vendor service slug must be at most 128 characters")
  .regex(
    /^[a-z0-9-]+$/,
    "Use only lowercase letters, digits, and hyphens",
  );

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
]);

export const pricingSyncStatusSchema = z.enum([
  "pending",
  "synced",
  "failed",
]);

/// `display` is rendered by the backend so a per-second or per-character
/// price cannot be relabelled as a per-call price by a client.
const pricingResponseSchema = z
  .object({
    billable: z.boolean(),
    metric: billingMetricSchema,
    price_per_unit: z.string().min(1),
    base_fee_per_call: z.string().min(1).nullable(),
    display: z.string().min(1),
  })
  .strict();

const adminPricingResponseSchema = z
  .object({
    billable: z.boolean(),
    metric: billingMetricSchema,
    price_per_unit: z.string().min(1),
    base_fee_per_call: z.string().min(1).nullable(),
    display: z.string().min(1),
    // Empty until the operation price has been synchronized to Lago.
    lago_metric_code: z.string(),
    sync_status: pricingSyncStatusSchema,
    sync_error: z.string().nullable(),
  })
  .strict();

const operationMetadataSchema = {
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
  })
  .strict();

export const speakConfigSchema = speakConfigResponseSchema.extend({
  allowed_voice_ids: uniqueStrings(
    safeIdentifierSchema("Voice ID"),
    100,
  ).min(1, "Add at least one allowed voice ID"),
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

export const platformOperationListSchema = z
  .object({
    operations: z.array(platformOperationSchema),
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
    credential_source: z.enum(["platform", "own_connection"]),
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
  typeof platformOperationListSchema
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
