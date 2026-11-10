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
/** Agent ids are opaque to the app; only the shape of a query value is checked. */
const AGENT_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;
const CARD_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;

export type NotificationTarget =
  /** A thread (`conversationId`), optionally focused on one decision card,
   * or an agent's latest thread when only `agentId` is known. */
  | {
      kind: "chat";
      conversationId?: string;
      agentId?: string;
      acknowledgementId?: string;
      openLatest?: boolean;
    }
  | { kind: "group"; groupId: string }
  /** The agent's details page, where its Connected channels are listed. */
  | { kind: "agent"; agentId: string }
  | { kind: "automation"; triggerId: string }
  | { kind: "automations"; agentId?: string }
  | { kind: "none" };

/**
 * Where a canonical server link points. Only a relative link or one on a
 * first-party frontend origin is read; anything else (another origin,
 * credentials, a fragment, an unknown page) yields null so the caller can
 * fall back to the row's own references. Mirrors `AssistantPage` on the
 * server: `?c=` names a thread or crew, `?card=` a decision card,
 * `?agent=&details=` an agent's details, `/assistant/automations` the
 * automations list.
 */
export function notificationLinkTarget(
  href: string | null | undefined,
  frontendUrls: string | readonly string[],
): NotificationTarget | null {
  if (!href) return null;
  const origins: string[] = [];
  for (const value of typeof frontendUrls === "string" ? [frontendUrls] : frontendUrls) {
    try {
      const origin = new URL(value).origin;
      if (/^https?:$/.test(new URL(value).protocol) && !origins.includes(origin))
        origins.push(origin);
    } catch {
      // Unset or malformed configured origins trust nothing.
    }
  }
  let url: URL;
  try {
    // Without a trusted origin only relative links can be read; an absolute
    // link then resolves to the placeholder origin and is rejected below.
    url = new URL(href, origins[0] ?? "https://native.nyxid.invalid/");
  } catch {
    return null;
  }
  if (
    !/^https?:$/.test(url.protocol) ||
    !origins.includes(url.origin) ||
    url.username ||
    url.password ||
    url.hash
  )
    return null;
  const search = url.searchParams;
  if (/^\/(?:assistant\/)?automations\/?$/.test(url.pathname)) {
    const agentId = search.get("agent") ?? search.get("agentId");
    return agentId && AGENT_ID.test(agentId)
      ? { kind: "automations", agentId }
      : { kind: "automations" };
  }
  if (!/^\/assistant\/?$/.test(url.pathname)) return null;
  if (search.has("panel")) return null;
  const c = search.get("c");
  const agentId = search.get("agent");
  if (c) {
    if (GROUP_ID.test(c)) return { kind: "group", groupId: c };
    if (!CONVERSATION_ID.test(c)) return null;
    const card = search.get("card");
    return {
      kind: "chat",
      conversationId: c,
      ...(agentId && AGENT_ID.test(agentId) ? { agentId } : {}),
      ...(card && CARD_ID.test(card) ? { acknowledgementId: card } : {}),
    };
  }
  const g = search.get("g");
  if (g) return GROUP_ID.test(g) ? { kind: "group", groupId: g } : null;
  if (!agentId || !AGENT_ID.test(agentId)) return null;
  if (search.has("details")) return { kind: "agent", agentId };
  return { kind: "chat", agentId, openLatest: true };
}

/** The destination a row's own references name, without reading its link. */
export function notificationRefsTarget(
  source: AssistantNotification["source"],
): NotificationTarget {
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

/**
 * Where a tap goes. The server's canonical link decides first, so a row
 * whose references name nothing the app can open (an organization bot's
 * public transcript, keyed by a hidden thread) still lands on its exact
 * page. The references remain the fallback for links the app cannot read
 * and refine a generic automations link to the run's own trigger.
 */
export function notificationTarget(
  item: Pick<AssistantNotification, "source"> & { url?: string | null },
  frontendUrls: string | readonly string[] = [],
): NotificationTarget {
  const refs = notificationRefsTarget(item.source);
  const link = notificationLinkTarget(item.url, frontendUrls);
  if (!link || link.kind === "none") return refs;
  if (link.kind === "automations" && refs.kind === "automation") return refs;
  if (link.kind === "chat" && !link.agentId && refs.kind === "chat" && refs.agentId)
    return { ...link, agentId: refs.agentId };
  return link;
}

const GROUP_TYPES: readonly NotificationSourceType[] = [
  "group",
  "collaboration_group",
];

/** Unread counts per visible thread and crew, derived once from a summary. */
export type NotificationMarkers = {
  conversations: Readonly<Record<string, number>>;
  groups: Readonly<Record<string, number>>;
};
export const EMPTY_NOTIFICATION_MARKERS: NotificationMarkers = Object.freeze({
  conversations: Object.freeze({}),
  groups: Object.freeze({}),
});

/**
 * The summary attributes unread rows to sources. A thread owns its own rows
 * plus the channel and automation rows that name it as their
 * `conversation_id`, matching what a source read on that thread clears; a
 * crew owns its group and collaboration rows. Built once per summary so
 * every sidebar row reads a map instead of scanning.
 */
export function notificationMarkers(
  summary: Pick<NotificationSummary, "by_source"> | undefined,
): NotificationMarkers {
  if (!summary?.by_source.length) return EMPTY_NOTIFICATION_MARKERS;
  const conversations: Record<string, number> = {};
  const groups: Record<string, number> = {};
  const add = (map: Record<string, number>, id: string, unread: number) => {
    if (unread > 0) map[id] = (map[id] ?? 0) + unread;
  };
  for (const entry of summary.by_source) {
    if (GROUP_TYPES.includes(entry.source_type)) {
      add(groups, entry.id, entry.unread);
      continue;
    }
    if (entry.conversation_id) add(conversations, entry.conversation_id, entry.unread);
    if (entry.source_type === "conversation" && entry.id !== entry.conversation_id)
      add(conversations, entry.id, entry.unread);
  }
  return { conversations, groups };
}

/** Unread rows the summary attributes to one visible thread or group. */
export function unreadForSource(
  summary: Pick<NotificationSummary, "by_source"> | undefined,
  type: "conversation" | "group",
  id: string,
): number {
  const markers = notificationMarkers(summary);
  return (type === "group" ? markers.groups : markers.conversations)[id] ?? 0;
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
