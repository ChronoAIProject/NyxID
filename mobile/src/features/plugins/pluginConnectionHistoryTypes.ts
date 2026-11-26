import { localizedError } from "../../i18n/localizedError";
import { translateApp } from "../../i18n/appTranslation";
import { z } from "zod";

const actorSchema = z.object({
  kind: z.string(),
  id: z.string(),
  name: z.string(),
  person_id: z.string().nullable(),
  api_key_id: z.string().nullable(),
  app_id: z.string().nullable(),
});
export const connectionHistorySchema = z.object({
  service_id: z.string(),
  next_cursor: z.string().nullable(),
  tracked_since: z.string().nullable(),
  legacy: z.boolean(),
  deleted: z.boolean(),
  groups: z.array(
    z.object({
      id: z.string(),
      events: z.array(
        z.object({
          id: z.string(),
          schema_version: z.number().int(),
          action: z.string(),
          action_label: z.string().optional(),
          actor: actorSchema,
          committed_at: z.string(),
          additional_changes: z.boolean(),
          audit_status: z.enum(["pending", "published", "mismatch"]),
          changes: z.array(
            z.object({
              field: z.string(),
              label: z.string().optional(),
              before: z.unknown().nullable(),
              after: z.unknown().nullable(),
            }),
          ),
        }),
      ),
    }),
  ),
});
export type ConnectionHistory = z.infer<typeof connectionHistorySchema>;
export type ConnectionHistoryGroup = ConnectionHistory["groups"][number];
export function parseConnectionHistory(
  value: unknown,
  serviceId: string,
  cursor: string | null,
) {
  const page = connectionHistorySchema.parse(value);
  if (page.service_id !== serviceId)
    throw localizedError("History returned for a different connection.");
  if (page.next_cursor !== null && page.next_cursor === cursor)
    throw localizedError("History returned an invalid cursor.");
  return page;
}

export const historyActionsSchema = z.object({
  option_set: z.literal("service-history-action"),
  items: z.array(
    z.object({ value: z.string(), label: z.string(), disabled: z.boolean() }),
  ),
  total: z.number().int().nonnegative(),
  next_offset: z.number().int().nonnegative().nullable(),
  version: z.string(),
});
export function parseHistoryActions(
  value: unknown,
  offset: number,
  version?: string,
) {
  const page = historyActionsSchema.parse(value);
  if (version !== undefined && page.version !== version)
    throw localizedError("History filters changed. Reload the choices.");
  if (
    page.next_offset !== null &&
    (page.next_offset <= offset ||
      page.next_offset >= page.total ||
      page.next_offset > 1000)
  )
    throw localizedError("History filters returned invalid pagination.");
  return page;
}

/** Match Web's metadata presentation; arbitrary objects must never be serialized. */
export function historyValue(field: string, value: unknown): string {
  if (value == null) return translateApp("Not recorded");
  if (typeof value === "boolean") return value ? translateApp("Yes") : translateApp("No");
  if (typeof value === "string" || typeof value === "number")
    return String(value);
  if (
    field === "token_scopes" &&
    Array.isArray(value) &&
    value.every((item) => typeof item === "string")
  )
    return value.length ? value.join(", ") : translateApp("No reviewed scopes");
  if (
    (field === "default_request_headers" || field === "ws_frame_injections") &&
    typeof value === "object" &&
    !Array.isArray(value)
  ) {
    const metadata = value as Record<string, unknown>;
    const items =
      metadata[field === "default_request_headers" ? "names" : "directions"];
    if (
      typeof metadata.count === "number" &&
      Array.isArray(items) &&
      items.every((item) => typeof item === "string")
    ) {
      const noun = field === "default_request_headers" ? "header" : "rule";
      return `${metadata.count} ${noun}${metadata.count === 1 ? "" : "s"}${items.length ? ` (${items.join(", ")})` : ""}`;
    }
  }
  return translateApp("Changed");
}
export function historyChange(
  change: ConnectionHistoryGroup["events"][number]["changes"][number],
) {
  return JSON.stringify(change.before) === JSON.stringify(change.after)
    ? translateApp("Changed; values omitted")
    : `${historyValue(change.field, change.before)} → ${historyValue(change.field, change.after)}`;
}
export function historySummary(group: ConnectionHistoryGroup) {
  return (
    group.events.find((event) => event.action === "service.deleted") ??
    group.events.find((event) => event.action === "service.created") ??
    group.events.at(-1)
  );
}
export function historyActor(actor: z.infer<typeof actorSchema>) {
  const kinds: Record<string, string> = {
    api_key: translateApp("API key"),
    service_account: translateApp("Service account"),
    app: translateApp("Application"),
    system: translateApp("System"),
  };
  return Object.hasOwn(kinds, actor.kind)
    ? `${actor.name} (${kinds[actor.kind]})`
    : actor.name;
}
