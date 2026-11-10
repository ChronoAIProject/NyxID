import { formatRelativeTimeFromMs } from "../../lib/time";
import {
  isNotificationKind,
  type AssistantNotification,
  type NotificationKind,
  type NotificationSourceType,
  type NotificationSummary,
} from "./notificationContracts";

export const NOTIFICATION_KIND_LABEL: Record<NotificationKind, string> = {
  automation_result: "Automation finished",
  automation_problem: "Automation needs attention",
  needs_decision: "Needs your decision",
  agent_update: "Agent update",
  chat_update: "Chat update",
  group_mention: "Mentioned you",
};

export type NotificationTone = "default" | "warning" | "danger" | "info";
export function notificationTone(kind: NotificationKind): NotificationTone {
  switch (kind) {
    case "automation_problem":
      return "danger";
    case "needs_decision":
      return "warning";
    case "group_mention":
      return "info";
    default:
      return "default";
  }
}

export function notificationBellLabel(
  unread: number,
  needsDecision: number,
): string {
  let label = "Notifications";
  if (unread > 0) label += `, ${unread} unread`;
  if (needsDecision > 0)
    label += `, ${needsDecision} ${needsDecision === 1 ? "needs" : "need"} your decision`;
  return label;
}

export const NOTIFICATION_BADGE_CAP = 9;
/** Bell badge text: empty when nothing is unread, capped at "9+". */
export function notificationBadge(unread: number): string {
  if (!Number.isFinite(unread) || unread <= 0) return "";
  return unread > NOTIFICATION_BADGE_CAP ? `${NOTIFICATION_BADGE_CAP}+` : String(Math.floor(unread));
}

/** Coalesced rows say how many updates arrived since the row was opened. */
export function notificationCountLabel(count: number): string | null {
  return Number.isFinite(count) && count > 1 ? `${Math.floor(count)} new` : null;
}

export function notificationTitle(
  item: Pick<AssistantNotification, "title" | "kind" | "source">,
): string {
  const title = item.title.trim();
  if (title) return title;
  const label = item.source.label?.trim();
  return label || NOTIFICATION_KIND_LABEL[item.kind];
}

export function notificationTime(iso: string, now = Date.now()): string {
  const at = Date.parse(iso);
  return Number.isFinite(at) ? formatRelativeTimeFromMs(at, now, "") : "";
}

/** A decision card is still waiting for the owner. */
export function notificationPending(
  item: Pick<AssistantNotification, "kind" | "resolved">,
): boolean {
  return item.kind === "needs_decision" && !item.resolved;
}

export function notificationAccessibilityLabel(
  item: AssistantNotification,
  now = Date.now(),
): string {
  const parts = [notificationTitle(item)];
  if (item.preview) parts.push(item.preview);
  const time = notificationTime(item.updated_at, now);
  if (time) parts.push(time);
  const count = notificationCountLabel(item.count);
  if (count) parts.push(count);
  if (notificationPending(item)) parts.push("needs your decision");
  if (!item.read) parts.push("unread");
  return parts.join(", ");
}

const CONVERSATION_ID = /^nyxa-[a-f0-9]{32}$/;
const GROUP_ID = /^nyxg-[a-fA-F0-9]{32}$/;
const UUID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export type NotificationTarget =
  | { kind: "chat"; conversationId: string; agentId?: string }
  | { kind: "group"; groupId: string }
  | { kind: "automation"; triggerId: string }
  | { kind: "automations" }
  | { kind: "none" };

/** Where a tap goes: the thread when one exists, else the group or automation. */
export function notificationTarget(
  item: Pick<AssistantNotification, "source">,
): NotificationTarget {
  const source = item.source;
  // Conversation and channel-chat rows are keyed by their thread id.
  const conversationId =
    source.conversation_id ??
    (source.type === "conversation" || source.type === "channel_chat"
      ? source.id
      : null);
  if (conversationId && CONVERSATION_ID.test(conversationId))
    return {
      kind: "chat",
      conversationId,
      agentId: source.agent_id ?? undefined,
    };
  if (source.type === "group" || source.type === "collaboration_group") {
    const groupId = source.group_id ?? source.id;
    if (GROUP_ID.test(groupId)) return { kind: "group", groupId };
  }
  if (source.type === "trigger") {
    const triggerId = source.trigger_id ?? source.id;
    return UUID.test(triggerId)
      ? { kind: "automation", triggerId }
      : { kind: "automations" };
  }
  return { kind: "none" };
}

const GROUP_TYPES: readonly NotificationSourceType[] = [
  "group",
  "collaboration_group",
];

/**
 * Unread rows the summary attributes to one visible thread or group. A
 * thread also owns the channel and automation rows that name it as their
 * `conversation_id`, matching what a source read on that thread clears.
 */
export function unreadForSource(
  summary: Pick<NotificationSummary, "by_source"> | undefined,
  type: "conversation" | "group",
  id: string,
): number {
  if (!summary) return 0;
  return summary.by_source.reduce((total, entry) => {
    const matches =
      type === "group"
        ? GROUP_TYPES.includes(entry.source_type) && entry.id === id
        : entry.conversation_id === id ||
          (entry.source_type === "conversation" && entry.id === id);
    return matches ? total + entry.unread : total;
  }, 0);
}

function asNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/** The closed kind a push payload names, or null for every other push. */
export function notificationKindFromPushData(
  data: unknown,
): NotificationKind | null {
  if (!data || typeof data !== "object") return null;
  const type = asNonEmptyString((data as { type?: unknown }).type);
  return isNotificationKind(type) ? type : null;
}

/** The notification a push tap identifies, so the app can mark it read. */
export function notificationIdFromPushData(data: unknown): string | null {
  if (!data || typeof data !== "object") return null;
  const payload = data as { notification_id?: unknown; notificationId?: unknown };
  return (
    asNonEmptyString(payload.notification_id) ??
    asNonEmptyString(payload.notificationId)
  );
}
