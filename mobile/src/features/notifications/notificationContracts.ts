import { z } from "zod";

/** Closed notification kinds; the server never sends anything else (SPEC D3). */
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

export function isNotificationKind(value: unknown): value is NotificationKind {
  return (
    typeof value === "string" &&
    (NOTIFICATION_KINDS as readonly string[]).includes(value)
  );
}

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

const instantSchema = z.string().datetime({ offset: true });
const optionalIdSchema = z.string().min(1).nullable().default(null);

export const notificationSourceSchema = z.object({
  type: notificationSourceTypeSchema,
  id: z.string().min(1),
  label: z.string().nullable().default(null),
  conversation_id: optionalIdSchema,
  group_id: optionalIdSchema,
  channel_id: optionalIdSchema,
  trigger_id: optionalIdSchema,
  agent_id: optionalIdSchema,
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

/** One feed row. Previews are server-rendered, single line, bounded text. */
export const notificationSchema = z.object({
  id: z.string().min(1),
  kind: notificationKindSchema,
  count: z.number().int().min(0).default(1),
  source: notificationSourceSchema,
  actor: notificationActorSchema.optional(),
  platform: z.string().nullable().optional(),
  participants: notificationActorSchema.array().max(3).optional(),
  /** Stable terminal codes; action wording belongs to the client. */
  outcome: z.string().nullable().optional(),
  reason: z.string().nullable().optional(),
  title: z.string().default(""),
  preview: z.string().default(""),
  url: z.string().nullable().default(null),
  read: z.boolean().default(false),
  resolved: z.boolean().default(false),
  created_at: instantSchema,
  updated_at: instantSchema,
});
export type AssistantNotification = z.infer<typeof notificationSchema>;

export const notificationPageSchema = z.object({
  /** Rows parse one by one: a row an older app cannot read (a kind added
   * later, a reshaped source) is dropped, never the whole page. */
  items: z.array(z.unknown()).transform((rows) =>
    rows.flatMap((row) => {
      const parsed = notificationSchema.safeParse(row);
      return parsed.success ? [parsed.data] : [];
    }),
  ),
  next_cursor: z.string().nullable().default(null),
  /** Server list-start time: the fence for "mark all as read". */
  as_of: instantSchema,
});
export type NotificationPage = z.infer<typeof notificationPageSchema>;

export const notificationSourceCountSchema = z.object({
  source_type: notificationSourceTypeSchema,
  id: z.string().min(1),
  /** The thread a channel or automation row belongs to, when it has one. */
  conversation_id: optionalIdSchema,
  unread: z.number().int().min(0),
});
export const notificationSummarySchema = z.object({
  /** Same live visibility rules as unread_count; one count per feed row. */
  by_kind: z
    .partialRecord(notificationKindSchema, z.number().int().nonnegative())
    .optional(),
  unread_count: z.number().int().min(0).default(0),
  needs_decision_count: z.number().int().min(0).default(0),
  /** Bounded (200 entries); `by_source_truncated` says more sources exist. */
  by_source: notificationSourceCountSchema.array().default([]),
  by_source_truncated: z.boolean().default(false),
});
export type NotificationSummary = z.infer<typeof notificationSummarySchema>;

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
export const DEFAULT_NOTIFICATION_PUSH: NotificationPushPreferences = {
  automation_result: false,
  automation_problem: false,
  needs_decision: false,
  agent_update: false,
  chat_update: false,
  group_mention: false,
};
/** The additive `notifications` slice of `GET /assistant/nyxagent/settings`.
 * Older servers omit it entirely; every push switch then reads as off. */
export const notificationSettingsSliceSchema = z.object({
  notifications: z
    .object({ push: notificationPushPreferencesSchema.optional() })
    .nullable()
    .optional(),
});
export function notificationPushPreferences(
  settings: unknown,
): NotificationPushPreferences {
  const slice = notificationSettingsSliceSchema.parse(settings);
  return { ...DEFAULT_NOTIFICATION_PUSH, ...(slice.notifications?.push ?? {}) };
}

export const MAX_NOTIFICATION_READ_IDS = 100;
export type NotificationReadScope =
  | { ids: readonly string[] }
  | { source: { type: NotificationSourceType; id: string } }
  | { all: true; before: string };

/** Exactly one of `ids`, `source` or `all`+`before` (SPEC D12). */
export function notificationReadBody(
  scope: NotificationReadScope,
): Record<string, unknown> {
  if ("ids" in scope) {
    const ids = [...new Set(scope.ids.filter((id) => id.length > 0))];
    if (ids.length === 0)
      throw new Error("Choose at least one notification to mark as read.");
    if (ids.length > MAX_NOTIFICATION_READ_IDS)
      throw new Error(
        `Mark at most ${MAX_NOTIFICATION_READ_IDS} notifications at once.`,
      );
    return { ids };
  }
  if ("source" in scope) {
    notificationSourceTypeSchema.parse(scope.source.type);
    if (!scope.source.id) throw new Error("A source needs an identifier.");
    return { source: { type: scope.source.type, id: scope.source.id } };
  }
  instantSchema.parse(scope.before);
  return { all: true, before: scope.before };
}

export const NOTIFICATION_FILTERS = [
  "all",
  "needs_you",
  "automations",
  "chats",
] as const;
export type NotificationFilter = (typeof NOTIFICATION_FILTERS)[number];
export const NOTIFICATION_FILTER_LABELS: Record<NotificationFilter, string> = {
  all: "All",
  needs_you: "Needs you",
  automations: "Automations",
  chats: "Chats",
};

/** The closed kind subset a filter asks the server for; `All` asks for none. */
export function notificationFilterKinds(
  filter: NotificationFilter,
): readonly NotificationKind[] | undefined {
  switch (filter) {
    case "needs_you":
      return ["needs_decision"];
    case "automations":
      return ["automation_result", "automation_problem"];
    case "chats":
      return ["chat_update", "agent_update", "group_mention"];
    default:
      return undefined;
  }
}

/** Needs you lists pending decisions, seen or not; a resolved card is done. */
export function notificationFilterKeeps(
  filter: NotificationFilter,
  item: Pick<AssistantNotification, "kind" | "resolved">,
): boolean {
  if (filter === "needs_you")
    return item.kind === "needs_decision" && !item.resolved;
  const kinds = notificationFilterKinds(filter);
  return !kinds || kinds.includes(item.kind);
}
