import { z } from "zod";

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
/** Only the saved timezone is needed to display automation history. */
export const automationSettingsSchema = z.object({
  timezone: automationTimezoneSchema
    .nullish()
    .transform((value) => value ?? null),
});

export function automationRunsPath(id: string, cursor?: AutomationRunsCursor) {
  const path = `/triggers/${automationIdSchema.parse(id)}/runs`;
  return cursor
    ? `${path}?${new URLSearchParams(automationRunsCursorSchema.parse(cursor))}`
    : path;
}
