import { z } from "zod";

// Public protocol fallbacks from backend/src/models/assistant_settings.rs.
// Saved owner settings take precedence over these rolling-upgrade defaults.
export const DEFAULT_SCHEDULE_MINIMUM_MINUTES = 5;
export const SCHEDULE_MINIMUM_MINUTES_LIMIT = 1440;
export const DEFAULT_TRIGGER_RUNS_PER_HOUR = 30;
export const TRIGGER_RUNS_PER_HOUR_LIMIT = 300;
export const DEFAULT_TRIGGER_RUNS_PER_DAY = 300;
export const TRIGGER_RUNS_PER_DAY_LIMIT = 3000;

export function utf8Length(value: string): number {
  let length = 0;
  for (const character of value) {
    const code = character.codePointAt(0)!;
    length += code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
  }
  return length;
}

export const automationIdSchema = z.string().uuid();
export const automationReferenceSchema = z
  .string()
  .min(1)
  .max(256)
  .refine(
    (value) => !/[\s\u0000-\u001f\u007f]/.test(value),
    "Enter a valid reference",
  );
export const automationInstantSchema = z.string().datetime({ offset: true });
export const automationTimezoneSchema = z
  .string()
  .trim()
  .min(1)
  .max(100)
  .refine((value) => {
    try {
      new Intl.DateTimeFormat("en", { timeZone: value });
      return true;
    } catch {
      return false;
    }
  }, "Enter an IANA timezone");
const scheduleOptions = {
  start: automationInstantSchema.nullish(),
  end: automationInstantSchema.nullish(),
  max_runs: z.number().int().min(1).max(1_000_000).nullish(),
  grace_seconds: z.number().int().min(1).max(86_400).nullish(),
};
export const scheduleSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("cron"),
    expression: z.string().min(1).max(256),
    timezone: automationTimezoneSchema,
    ...scheduleOptions,
  }),
  z.object({
    kind: z.literal("every"),
    amount: z.number().int().min(1).max(4_294_967_295),
    unit: z.enum(["minutes", "hours", "days"]),
    anchor: automationInstantSchema,
    ...scheduleOptions,
  }),
  z.object({
    kind: z.literal("at"),
    at: automationInstantSchema,
    ...scheduleOptions,
  }),
]);
export type ScheduleSpec = z.infer<typeof scheduleSchema>;

export const automationStatusSchema = z.enum(["active", "disabled"]);
export const automationVerificationSchema = z.discriminatedUnion("mode", [
  z.object({ mode: z.literal("schedule") }),
  z.object({ mode: z.literal("token"), location: z.enum(["bearer", "query"]) }),
  z.object({
    mode: z.literal("hmac_sha256"),
    header_name: z.string().min(1).max(128),
  }),
]);
export const deliverToSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("thread") }),
  z.object({ type: z.literal("notification") }),
  z.object({ type: z.literal("chat"), chat_id: automationReferenceSchema }),
]);
export const automationDeliverySchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("assistant"),
    agent_id: automationReferenceSchema,
    thread_policy: z.enum(["home", "dedicated", "new"]).nullish(),
    instruction: z
      .string()
      .min(1)
      .refine(
        (value) => !!value.trim() && utf8Length(value) <= 8192,
        "Enter an instruction of up to 8 KB",
      ),
    confirmation_policy: z.enum(["changes", "destructive"]).optional(),
    deliver_to: deliverToSchema,
  }),
  z.object({ type: z.literal("notification") }),
  z.object({ type: z.literal("webhook"), url: z.string().url() }),
  z.object({
    type: z.literal("agent"),
    conversation_id: automationReferenceSchema,
  }),
]);
export type AutomationDelivery = z.infer<typeof automationDeliverySchema>;

export const automationRunSchema = z.object({
  id: automationIdSchema,
  scheduled_at: automationInstantSchema,
  started_at: automationInstantSchema.nullable(),
  finished_at: automationInstantSchema.nullable(),
  outcome: z.enum([
    "pending",
    "started",
    "waiting",
    "completed",
    "failed",
    "skipped",
  ]),
  missed: z
    .object({
      count: z.number().int().nonnegative(),
      from: automationInstantSchema,
      through: automationInstantSchema,
    })
    .nullish(),
  reason: z.string().nullable(),
  // Non-assistant scheduler deliveries persist an empty thread sentinel.
  // Normalize it so consumers offer thread navigation only for real threads.
  thread_id: z
    .union([automationReferenceSchema, z.literal("")])
    .nullable()
    .transform((value) => (value === "" ? null : value)),
  duration_ms: z.number().int().nonnegative().nullable(),
});
export type AutomationRun = z.infer<typeof automationRunSchema>;
export const automationRunsCursorSchema = z
  .object({
    before: automationInstantSchema,
    before_id: automationIdSchema,
  })
  .strict();
export type AutomationRunsCursor = z.infer<typeof automationRunsCursorSchema>;
export const automationRunsSchema = z.object({
  runs: z.array(automationRunSchema).max(100),
  next_cursor: automationRunsCursorSchema.nullish(),
});
export type AutomationRunsPage = z.infer<typeof automationRunsSchema>;

export const automationSchema = z.object({
  id: automationIdSchema,
  user_id: automationReferenceSchema,
  label: z.string().min(1),
  user_service_id: automationReferenceSchema.nullable(),
  status: automationStatusSchema,
  source: z.enum(["webhook", "schedule"]).optional(),
  schedule: scheduleSchema.nullish(),
  overlap: z.enum(["skip", "queue"]).optional(),
  last_run: automationRunSchema.nullish(),
  next_runs: z.array(automationInstantSchema).optional(),
  next_run_at: automationInstantSchema.nullish(),
  pause_reason: z.string().nullish(),
  verification: automationVerificationSchema,
  delivery: automationDeliverySchema,
  inbound_url: z.string().url().nullable(),
  created_at: automationInstantSchema,
  updated_at: automationInstantSchema,
});
export type Automation = z.infer<typeof automationSchema>;
export const automationListSchema = z.object({
  triggers: z.array(automationSchema),
});
export const automationCreateResponseSchema = z.object({
  trigger: automationSchema,
  secret: z.string().min(1).nullable(),
  delivery_signing_secret: z.string().nullable(),
});
export const automationUpdateResponseSchema = z.object({
  trigger: automationSchema,
  delivery_signing_secret: z.string().nullable(),
});
export const automationDeleteResponseSchema = z.object({ message: z.string() });

const labelSchema = z
  .string()
  .trim()
  .min(1, "Enter a label")
  .max(128)
  .refine(
    (value) => utf8Length(value) <= 128,
    "Keep the label within 128 bytes",
  );
function validateDelivery(
  delivery: AutomationDelivery,
  context: z.RefinementCtx,
) {
  if (delivery.type === "webhook") {
    const url = new URL(delivery.url);
    if (url.protocol !== "https:" || url.username || url.password) {
      context.addIssue({
        code: "custom",
        path: ["delivery", "url"],
        message: "Enter a public HTTPS URL without credentials",
      });
    }
  }
}
export function validateSchedule(
  schedule: ScheduleSpec,
  context: z.RefinementCtx,
) {
  if (
    schedule.kind === "cron" &&
    schedule.expression.trim().split(/\s+/).length !== 5
  )
    context.addIssue({
      code: "custom",
      path: ["schedule", "expression"],
      message: "Cron needs five fields",
    });
  if (
    schedule.start &&
    schedule.end &&
    Date.parse(schedule.end) <= Date.parse(schedule.start)
  )
    context.addIssue({
      code: "custom",
      path: ["schedule", "end"],
      message: "Schedule end must be after start",
    });
}
export const automationCreateRequestSchema = z
  .object({
    source: z.enum(["webhook", "schedule"]).optional(),
    schedule: scheduleSchema.optional(),
    overlap: z.enum(["skip", "queue"]).optional(),
    watch_id: automationIdSchema.optional(),
    label: labelSchema,
    user_service_id: automationIdSchema.optional(),
    verification: automationVerificationSchema,
    delivery: automationDeliverySchema,
    target_org_id: automationIdSchema.optional(),
  })
  .strict()
  .superRefine((value, context) => {
    if (
      value.source === "schedule" &&
      (!value.schedule || value.verification.mode !== "schedule")
    )
      context.addIssue({
        code: "custom",
        path: ["schedule"],
        message: "Choose a schedule",
      });
    if (
      value.source !== "schedule" &&
      (value.schedule || value.verification.mode === "schedule")
    )
      context.addIssue({
        code: "custom",
        path: ["source"],
        message: "Webhook verification must use a token or HMAC",
      });
    if (
      value.verification.mode === "hmac_sha256" &&
      !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(value.verification.header_name)
    )
      context.addIssue({
        code: "custom",
        path: ["verification", "header_name"],
        message: "Enter a valid HTTP header name",
      });
    if (value.schedule) validateSchedule(value.schedule, context);
    validateDelivery(value.delivery, context);
  });
export type AutomationCreateRequest = z.infer<
  typeof automationCreateRequestSchema
>;
export const automationUpdateRequestSchema = z
  .object({
    schedule: scheduleSchema.optional(),
    overlap: z.enum(["skip", "queue"]).optional(),
    label: labelSchema.optional(),
    status: automationStatusSchema.optional(),
    delivery: automationDeliverySchema.optional(),
  })
  .strict()
  .superRefine((value, context) => {
    if (value.schedule) validateSchedule(value.schedule, context);
    if (value.delivery) validateDelivery(value.delivery, context);
  });
export type AutomationUpdateRequest = z.infer<
  typeof automationUpdateRequestSchema
>;

export const automationPreviewSchema = z.object({
  next_runs: z.array(automationInstantSchema),
  description: z.string().optional(),
});
export const automationSetupSchema = z.object({
  label: z.string(),
  instruction: z.string(),
  agent_id: automationReferenceSchema,
  confirmation_policy: z.enum(["changes", "destructive"]),
  thread_policy: z.enum(["home", "dedicated", "new"]).nullish(),
});
export type AutomationSetup = z.infer<typeof automationSetupSchema>;
export const automationChatSchema = z.object({
  id: automationReferenceSchema,
  channel_agent_id: automationReferenceSchema,
  platform: z.string(),
  bot_label: z.string(),
  title: z.string().nullable().default(null),
  kind: z.string().nullable().default(null),
  agent_id: automationReferenceSchema.nullable().default(null),
  allow_posts: z.boolean().default(false),
  conversation_id: automationReferenceSchema.nullable().default(null),
});
export type AutomationChat = z.infer<typeof automationChatSchema>;

export const automationDeviceConversationSchema = z.object({
  id: automationIdSchema,
  platform: z.string().min(1),
  platform_conversation_id: z.string().min(1),
  platform_conversation_type: z.string().min(1),
  is_active: z.boolean(),
  allow_agent_initiated: z.boolean(),
});
export type AutomationDeviceConversation = z.infer<
  typeof automationDeviceConversationSchema
>;

const preferenceFields = {
  schedule_minimum_minutes: z
    .number()
    .int()
    .min(DEFAULT_SCHEDULE_MINIMUM_MINUTES)
    .max(SCHEDULE_MINIMUM_MINUTES_LIMIT),
  trigger_runs_per_hour: z
    .number()
    .int()
    .min(1)
    .max(TRIGGER_RUNS_PER_HOUR_LIMIT),
  trigger_runs_per_day: z.number().int().min(1).max(TRIGGER_RUNS_PER_DAY_LIMIT),
};
export const automationPreferencesSchema = z
  .object({ timezone: automationTimezoneSchema, ...preferenceFields })
  .strict();
export type AutomationPreferences = z.infer<typeof automationPreferencesSchema>;
export const automationSettingsSchema = z.object({
  timezone: automationTimezoneSchema
    .nullish()
    .transform((value) => value ?? null),
  schedule_minimum_minutes: preferenceFields.schedule_minimum_minutes.default(
    DEFAULT_SCHEDULE_MINIMUM_MINUTES,
  ),
  trigger_runs_per_hour: preferenceFields.trigger_runs_per_hour.default(
    DEFAULT_TRIGGER_RUNS_PER_HOUR,
  ),
  trigger_runs_per_day: preferenceFields.trigger_runs_per_day.default(
    DEFAULT_TRIGGER_RUNS_PER_DAY,
  ),
});
export type AutomationSettings = z.infer<typeof automationSettingsSchema>;
export const automationPreferencesUpdateSchema = z
  .object({
    timezone: automationTimezoneSchema.nullable().optional(),
    schedule_minimum_minutes:
      preferenceFields.schedule_minimum_minutes.optional(),
    trigger_runs_per_hour: preferenceFields.trigger_runs_per_hour.optional(),
    trigger_runs_per_day: preferenceFields.trigger_runs_per_day.optional(),
  })
  .strict();
export type AutomationPreferencesUpdate = z.infer<
  typeof automationPreferencesUpdateSchema
>;

export function automationRunsPath(id: string, cursor?: AutomationRunsCursor) {
  const path = `/triggers/${automationIdSchema.parse(id)}/runs`;
  return cursor
    ? `${path}?${new URLSearchParams(automationRunsCursorSchema.parse(cursor))}`
    : path;
}
