import { z } from "zod";

/**
 * NyxBot Notifications: the per-person feed of automation results and chat
 * updates (`/assistant/nyxagent/notifications`). Rows carry identifiers,
 * counts and read state; the server computes the preview at list time, so a
 * row never stores content.
 */
export const NOTIFICATION_KINDS = [
  "automation_result",
  "automation_problem",
  "needs_decision",
  "agent_update",
  "chat_update",
  "group_mention",
] as const;
export const notificationKindSchema = z.enum(NOTIFICATION_KINDS);
export type NotificationKind = z.infer<typeof notificationKindSchema>;

export const NOTIFICATION_SOURCE_TYPES = [
  "conversation",
  "group",
  "channel_chat",
  "collaboration_group",
  "trigger",
] as const;
export const notificationSourceTypeSchema = z.enum(NOTIFICATION_SOURCE_TYPES);
export type NotificationSourceType = z.infer<
  typeof notificationSourceTypeSchema
>;

const optionalId = z.string().nullable().default(null);

export const notificationSourceSchema = z.object({
  type: notificationSourceTypeSchema,
  id: z.string(),
  label: z.string().default(""),
  /** Set for threads and channel chats (which are conversations too). */
  conversation_id: optionalId,
  group_id: optionalId,
  channel_id: optionalId,
  trigger_id: optionalId,
  agent_id: optionalId,
});
export type NotificationSource = z.infer<typeof notificationSourceSchema>;

/** Identity resolved after the source's live access checks. Older servers omit it. */
export const notificationActorSchema = z.object({
  type: z.enum(["agent", "automation", "person", "system"]),
  id: z.string().nullable().optional(),
  name: z.string(),
  display_name: z.string().nullable().optional(),
  kind: z.enum(["nyxbot", "specialist"]).nullable().optional(),
});
export type NotificationActor = z.infer<typeof notificationActorSchema>;

export const notificationSchema = z.object({
  id: z.string(),
  kind: notificationKindSchema,
  /** Coalesced activity on one source since the row was opened. */
  count: z.number().int().nonnegative().default(1),
  source: notificationSourceSchema,
  actor: notificationActorSchema.optional(),
  platform: z.string().nullable().optional(),
  participants: notificationActorSchema.array().max(3).optional(),
  /** Stable terminal codes; action wording belongs to the client. */
  outcome: z.string().nullable().optional(),
  reason: z.string().nullable().optional(),
  title: z.string().default(""),
  /** Single line, markdown stripped, at most 200 characters; computed by the server. */
  preview: z.string().default(""),
  /** The canonical NyxID page for the source (an `AssistantPage` URL). */
  url: z.string().default(""),
  read: z.boolean().default(false),
  /** Decisions resolve when their card is decided, expires or is superseded. */
  resolved: z.boolean().default(false),
  created_at: z.string(),
  updated_at: z.string(),
});
export type AssistantNotification = z.infer<typeof notificationSchema>;

/** A row a different server release shapes unexpectedly is dropped on its own; the page still renders. */
function keepValid<T>(schema: z.ZodType<T>) {
  return z
    .array(z.unknown())
    .default([])
    .transform((rows) =>
      rows.flatMap((row) => {
        const parsed = schema.safeParse(row);
        return parsed.success ? [parsed.data] : [];
      }),
    );
}

export const notificationListSchema = z.object({
  items: keepValid(notificationSchema),
  next_cursor: z.string().nullable().default(null),
  /** Server time at list start: the fence for "Mark all as read". */
  as_of: z.string(),
});
export type NotificationList = z.infer<typeof notificationListSchema>;

export const notificationSummarySourceSchema = z.object({
  source_type: notificationSourceTypeSchema,
  id: z.string(),
  unread: z.number().int().nonnegative().default(0),
  /** Additive: lets channel chats and automation threads map onto sidebar rows. */
  conversation_id: z.string().nullable().optional(),
});
export type NotificationSummarySource = z.infer<
  typeof notificationSummarySourceSchema
>;

export const notificationSummarySchema = z.object({
  /** Same live visibility rules as unread_count; one count per feed row. */
  by_kind: z
    .partialRecord(notificationKindSchema, z.number().int().nonnegative())
    .optional(),
  /** Unseen rows that are still active (pending decisions included). */
  unread_count: z.number().int().nonnegative().default(0),
  /** Unseen pending decisions; shown distinctly on the bell. */
  needs_decision_count: z.number().int().nonnegative().default(0),
  by_source: keepValid(notificationSummarySourceSchema),
  /** Additive: the bounded aggregate left sources out. */
  by_source_truncated: z.boolean().optional(),
});
export type NotificationSummary = z.infer<typeof notificationSummarySchema>;

/** Per-kind push switches in NyxBot settings; the in-app feed is always on. */
export const notificationPushPreferencesSchema = z.object({
  automation_result: z.boolean().default(false),
  automation_problem: z.boolean().default(false),
  needs_decision: z.boolean().default(false),
  agent_update: z.boolean().default(false),
  chat_update: z.boolean().default(false),
  group_mention: z.boolean().default(false),
});
export type NotificationPushPreferences = z.infer<
  typeof notificationPushPreferencesSchema
>;
/** The settings form: every switch is a required boolean (no server defaults to fill in). */
export const notificationPushFormSchema = z.object({
  automation_result: z.boolean(),
  automation_problem: z.boolean(),
  needs_decision: z.boolean(),
  agent_update: z.boolean(),
  chat_update: z.boolean(),
  group_mention: z.boolean(),
});
export const DEFAULT_NOTIFICATION_PUSH: NotificationPushPreferences = {
  automation_result: false,
  automation_problem: false,
  needs_decision: false,
  agent_update: false,
  chat_update: false,
  group_mention: false,
};

export const notificationSettingsSchema = z.object({
  push: notificationPushPreferencesSchema.default(DEFAULT_NOTIFICATION_PUSH),
});
export type NotificationSettings = z.infer<typeof notificationSettingsSchema>;

/** The panel's filter chips. `all` sends no kind filter. */
export const NOTIFICATION_FILTERS = [
  "all",
  "needs_you",
  "automations",
  "chats",
] as const;
export type NotificationFilter = (typeof NOTIFICATION_FILTERS)[number];
export const NOTIFICATION_FILTER_KINDS: Record<
  NotificationFilter,
  readonly NotificationKind[]
> = {
  all: [],
  needs_you: ["needs_decision"],
  automations: ["automation_result", "automation_problem"],
  chats: ["chat_update", "agent_update", "group_mention"],
};
export const NOTIFICATION_FILTER_LABEL: Record<NotificationFilter, string> = {
  all: "All",
  needs_you: "Needs you",
  automations: "Automations",
  chats: "Chats",
};

export function notificationFilter(
  value: unknown,
): NotificationFilter | undefined {
  return NOTIFICATION_FILTERS.find((filter) => filter === value);
}

/** Exactly one of the three read scopes; `before` fences "all" to what the user saw. */
export type NotificationReadRequest =
  | { readonly ids: readonly string[] }
  | {
      readonly source: {
        readonly type: NotificationSourceType;
        readonly id: string;
      };
    }
  | { readonly all: true; readonly before: string };

export const NOTIFICATION_READ_MAX_IDS = 100;
