import { z } from "zod";

export const PLATFORM_OPERATION_QUERY_KEY = [
  "admin",
  "platform-ops",
] as const;
export const PLATFORM_OPERATION_DISCOVERY_QUERY_KEY = [
  "platform-ops",
] as const;
export const PLATFORM_VENDOR_REQUIREMENTS_QUERY_KEY = [
  "admin",
  "platform-ops",
  "vendor-requirements",
] as const;
export const PLATFORM_VENDOR_TEMPLATES_QUERY_KEY = [
  "admin",
  "platform-ops",
  "vendor-templates",
] as const;

export const platformVendorSchema = z
  .string()
  .trim()
  .min(1, "Vendor key is required")
  .max(64, "Vendor key must be at most 64 characters")
  .regex(
    /^[a-z0-9][a-z0-9_-]*$/,
    "Use lowercase letters, digits, underscores, and hyphens",
  );

const existingPlatformVendorServiceSchema = z
  .object({
    id: z.string().min(1),
    name: z.string().min(1),
    auth_method: z.string(),
    auth_key_name: z.string(),
    service_category: z.string(),
    visibility: z.string(),
    is_active: z.literal(true),
  })
  .strict();

export const platformVendorRequirementSchema = z
  .object({
    id: z.string().min(1),
    vendor: platformVendorSchema,
    display_name: z.string().min(1),
    operation: z.string().min(1).nullable(),
    slug: z.string().regex(/^platform-[a-z0-9-]+$/),
    base_url: z.url(),
    auth_method: z.enum(["header", "bearer", "basic"]),
    auth_key_name: z.string().min(1).nullable(),
    service_category: z.literal("internal"),
    visibility: z.literal("public"),
    credential_label: z.string().min(1),
    credential_note: z.string().min(1),
    capability_summary: z.string().min(1),
    restriction_summary: z.string().min(1),
    is_active: z.boolean(),
    is_seeded: z.boolean(),
    existing_service: existingPlatformVendorServiceSchema.nullable(),
  })
  .strict();

export const platformVendorRequirementListSchema = z
  .object({
    vendors: z.array(platformVendorRequirementSchema),
  })
  .strict();

export const platformVendorProvisionSchema = z
  .object({
    vendor: platformVendorSchema,
    credential: z
      .string()
      .trim()
      .min(1, "Credential is required")
      .max(16_384, "Credential is too large"),
    note: z
      .string()
      .trim()
      .max(4_096, "Operator note must be at most 4096 characters"),
  })
  .strict();

export const platformVendorTemplateFormSchema = z
  .object({
    vendor: platformVendorSchema,
    display_name: z.string().trim().min(1).max(200),
    slug: z
      .string()
      .trim()
      .regex(
        /^platform-[a-z0-9-]+$/,
        "Slug must start with platform- and use lowercase letters, digits, and hyphens",
      )
      .max(100),
    base_url: z.url(),
    auth_method: z.enum(["header", "bearer", "basic"]),
    auth_key_name: z.string().trim().max(256).nullable(),
    credential_label: z.string().trim().min(1).max(120),
    credential_note: z.string().trim().min(1).max(4_096),
    operation: z.string().trim().max(64).nullable(),
    capability_summary: z.string().trim().min(1).max(4_096),
    restriction_summary: z.string().trim().min(1).max(4_096),
    is_active: z.boolean(),
  })
  .strict();

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

const operationMetadataSchema = {
  enabled: z.boolean(),
  vendor_service_slug: vendorServiceSlugSchema,
  updated_at: z.string().nullable(),
  updated_by: z.string().nullable(),
  vendor_service_id: z.string().min(1).nullable(),
  pricing: z
    .object({
      billable: z.boolean(),
      credits_per_call: z.string().min(1).nullable(),
      metric: z.literal("requests"),
    })
    .strict(),
};

export const xSearchConfigSchema = z
  .object({
    type: z.literal("x_search"),
    max_results_cap: z
      .number()
      .int("Maximum results must be an integer")
      .min(1, "Maximum results must be at least 1")
      .max(25, "Maximum results cannot exceed 25"),
  })
  .strict();

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

export const xSearchOperationSchema = z
  .object({
    op: z.literal("x_search"),
    ...operationMetadataSchema,
    config: xSearchConfigSchema,
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
  xSearchOperationSchema,
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

export const xSearchUpdateSchema = z
  .object({
    ...updateMetadataSchema,
    config: xSearchConfigSchema,
  })
  .strict();

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
    op: z.enum(["x_search", "speak", "call_and_say", "flight_search"]),
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
        reason: z.enum(["disabled", "node_routed", "unusable"]).nullable(),
      })
      .strict()
      .nullable(),
    pricing: z
      .object({
        billable: z.boolean(),
        credits_per_call: z.string().min(1).nullable(),
        metric: z.literal("requests"),
      })
      .strict(),
    mcp_tool: z.string().min(1),
  })
  .strict();

export const platformOperationDiscoveryListSchema = z
  .object({
    operations: z.array(platformOperationDiscoverySchema),
  })
  .strict();

export type PlatformOperation = z.infer<typeof platformOperationSchema>;
export type PlatformOperationList = z.infer<
  typeof platformOperationListSchema
>;
export type XSearchOperation = z.infer<typeof xSearchOperationSchema>;
export type SpeakOperation = z.infer<typeof speakOperationSchema>;
export type CallAndSayOperation = z.infer<typeof callAndSayOperationSchema>;
export type FlightSearchOperation = z.infer<typeof flightSearchOperationSchema>;
export type XSearchUpdate = z.infer<typeof xSearchUpdateSchema>;
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
  | { readonly op: "x_search"; readonly data: XSearchUpdate }
  | { readonly op: "speak"; readonly data: SpeakUpdate }
  | { readonly op: "call_and_say"; readonly data: CallAndSayUpdate }
  | { readonly op: "flight_search"; readonly data: FlightSearchUpdate };

export type PlatformVendor = z.infer<typeof platformVendorSchema>;
export type PlatformVendorRequirement = z.infer<
  typeof platformVendorRequirementSchema
>;
export type PlatformVendorRequirementList = z.infer<
  typeof platformVendorRequirementListSchema
>;
export type PlatformVendorProvision = z.infer<
  typeof platformVendorProvisionSchema
>;
export type PlatformVendorTemplateForm = z.infer<
  typeof platformVendorTemplateFormSchema
>;

export interface ProvisionPlatformVendorVariables {
  readonly requirement: PlatformVendorRequirement;
  readonly data: PlatformVendorProvision;
  readonly replaceServiceId?: string;
}

export type PlatformVendorTemplateInput = z.infer<
  typeof platformVendorTemplateFormSchema
>;
