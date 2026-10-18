import { z } from "zod";

export const setupTemplateSchema = z.enum([
  "telegram-assistant",
  "restaurant-starter",
]);
export type SetupTemplate = z.infer<typeof setupTemplateSchema>;
export const setupStateSchema = z.enum([
  "draft",
  "awaiting_identity",
  "collecting",
  "awaiting_approval",
  "provisioning",
  "awaiting_child_review",
  "ready_for_test",
  "active",
  "failed_retryable",
  "paused",
  "cancelled",
  "expired",
]);
export const setupStatus: Record<z.infer<typeof setupStateSchema>, string> = {
  draft: "Draft",
  awaiting_identity: "Account linking required",
  collecting: "Collecting details",
  awaiting_approval: "Waiting for your review",
  provisioning: "Preparing your bot",
  awaiting_child_review: "Review the created Telegram bot",
  ready_for_test: "Ready for customer test",
  active: "Active",
  failed_retryable: "Needs recovery",
  paused: "Paused",
  cancelled: "Cancelled",
  expired: "Expired",
};
const customerUrl = z
  .string()
  .max(2048)
  .refine((value) => {
    try {
      const url = new URL(value);
      return (
        url.protocol === "https:" &&
        !url.username &&
        !url.password &&
        !url.hash &&
        (!url.port || url.port === "443")
      );
    } catch {
      return false;
    }
  }, "Use a public HTTPS link without credentials or a fragment")
  .nullable();
export const setupFactsSchema = z.strictObject({
  name: z.string().trim().min(1).max(100),
  timezone: z.string().min(1).max(100),
  hours: z.string().max(2000),
  address: z.string().max(2000),
  menu_url: customerUrl,
  reservation_url: customerUrl,
  contact: z.string().max(2000),
  policies: z.string().max(4000),
  faq: z
    .array(
      z.strictObject({
        question: z.string().min(1).max(500),
        answer: z.string().min(1).max(2000),
      }),
    )
    .max(20),
  assistant_instructions: z.string().max(4000),
});
export type SetupFacts = z.infer<typeof setupFactsSchema>;
export const setupLimitsSchema = z.strictObject({
  daily_credits: z
    .string()
    .regex(/^\d+(?:\.\d{1,12})?$/)
    .refine((value) => {
      if (!/^\d+(?:\.\d{1,12})?$/.test(value)) return false;
      const [whole, fraction = ""] = value.split(".");
      const exact =
        BigInt(whole!) * 1_000_000_000_000n + BigInt(fraction.padEnd(12, "0"));
      return exact > 0n && exact <= 1_000_000_000_000_000n;
    }, "Daily credits must be greater than zero and at most 1000"),
  turns_per_hour: z.number().int().min(1).max(100),
  concurrent_turns: z.number().int().min(1).max(2),
  max_input_chars: z.number().int().min(256).max(8192),
  max_output_tokens: z.number().int().min(128).max(4096),
});
export const setupSchema = z.object({
  id: z.uuid(),
  template: setupTemplateSchema,
  template_version: z.number().int(),
  revision: z.number().int(),
  state: setupStateSchema,
  runtime_notice: z.string().nullable().optional(),
  effective_state: setupStateSchema.optional(),
  facts: setupFactsSchema,
  limits: setupLimitsSchema,
  agent_id: z.string().nullable(),
  channel_bot_id: z.string().nullable(),
  creation_request_id: z.string().nullable(),
  observed_telegram_bot_id: z.string().nullable(),
  publication_revision: z.number().nullable(),
  approved: z.boolean(),
  delivery_tested: z.boolean(),
  expires_at: z.string().nullable(),
  updated_at: z.string(),
});
export type PlatformSetup = z.infer<typeof setupSchema>;
export const bindingSchema = z.object({
  id: z.uuid(),
  generation: z.number().int(),
  platform_config_id: z.uuid(),
  manager_bot_id: z.string(),
  telegram_sender_id: z.string(),
  expires_at: z.string(),
});
export const bindingPreviewSchema = z.object({
  request_id: z.uuid(),
  manager_bot_id: z.string(),
  requesting_sender_id: z.string(),
  state: z.enum([
    "pending",
    "awaiting_chat_confirmation",
    "bound",
    "denied",
    "expired",
    "cancelled",
  ]),
  expires_at: z.string(),
  scope: z.string(),
});
export const guideConfigurationSchema = z.object({
  model_service_id: z.uuid(),
  model_id: z.string().min(1).max(200),
  limits: setupLimitsSchema,
  anonymous_daily_credits: setupLimitsSchema.shape.daily_credits,
  platform_turns_per_hour: z.number().int().min(1).max(1000),
  platform_concurrent_turns: z.number().int().min(1).max(32),
});
export const platformProfileSchema = z.object({
  guide: guideConfigurationSchema.nullish(),
  id: z.uuid(),
  channel_bot_id: z.uuid(),
  manager_bot_id: z.string(),
  execution_profile: z.enum(["platform_onboarding", "native_entry"]),
  platform: z
    .enum(["telegram", "discord", "slack", "whatsapp", "lark", "feishu", "x"])
    .optional(),
  transport: z.enum(["direct", "gateway"]).optional(),
  supported_transports: z.array(z.enum(["direct", "gateway"])).optional(),
  generation: z.number().int(),
  enabled: z.boolean(),
  funding_owner_user_id: z.uuid(),
  deleted_at: z.string().nullable(),
});
export type PlatformProfile = z.infer<typeof platformProfileSchema>;
export const platformProfilesSchema = z.object({
  profiles: z.array(platformProfileSchema),
  next_before_id: z.string().nullable(),
});
export const templatesSchema = z.array(
  z.object({
    id: setupTemplateSchema,
    version: z.number().int(),
    name: z.string(),
    description: z.string(),
    fields: z.array(z.string()),
    runtime_connector_grants: z.array(z.string()),
  }),
);
export function onboardingSearch(search: Record<string, unknown>): {
  template?: SetupTemplate;
  code?: string;
} {
  const template = setupTemplateSchema.safeParse(search.template);
  const code =
    typeof search.code === "string" && /^[A-Za-z0-9-]{8,12}$/.test(search.code)
      ? search.code
      : undefined;
  return {
    ...(template.success ? { template: template.data } : {}),
    ...(code ? { code } : {}),
  };
}
export function blankFacts(): SetupFacts {
  return {
    name: "",
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
    hours: "",
    address: "",
    menu_url: null,
    reservation_url: null,
    contact: "",
    policies: "",
    faq: [],
    assistant_instructions: "",
  };
}

export type OwnedBinding = z.infer<typeof bindingSchema>;

export const setupApprovalSchema = z.object({
  setup: setupSchema,
  digest: z.string().min(1),
  reviewed_revision: z.number().int(),
  request_revision: z.number().int().nullable(),
  child_bot_id: z.number().int().nullable(),
  child_username: z.string().nullable(),
  manager_revision: z.number().int().nullable(),
  agent_id: z.uuid(),
  creation_request_id: z.uuid(),
  model_service_id: z.uuid(),
  model_service_name: z.string().nullish(),
  model_id: z.string(),
  specialist_name: z.string().optional(),
  funding_owner_user_id: z.uuid(),
  funding_owner_name: z.string().nullish(),
  connector_grants: z.array(z.string()),
});
export type SetupApproval = z.infer<typeof setupApprovalSchema>;

export const gatewayCleanupTaskSchema = z.object({
  id: z.uuid(),
  remote_channel_id: z.string().nullable(),
  captured_version: z.number().int().nullable(),
});
export const gatewayCleanupReviewSchema = z.object({
  task_id: z.uuid(),
  profile_generation: z.number().int(),
  remote_channel_id: z.string(),
  captured_version: z.number().int(),
  observed_version: z.number().int().nullable(),
  remote_exists: z.boolean(),
  completed: z.boolean(),
});
export type GatewayCleanupReview = z.infer<typeof gatewayCleanupReviewSchema>;
