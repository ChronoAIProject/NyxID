import { formatRelativeTimeFromMs } from "../../lib/time";
import {
  isNotificationKind,
  type AssistantNotification,
  type NotificationKind,
  type NotificationFilter,
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

export type NotificationTone = "default" | "warning" | "danger" | "success";
export function notificationTone(kind: NotificationKind): NotificationTone {
  switch (kind) {
    case "automation_problem":
      return "danger";
    case "needs_decision":
      return "warning";
    case "automation_result":
      return "success";
    default:
      return "default";
  }
}

export function notificationBellLabel(
  unread: number,
  needsDecision: number,
): string {
  let label = "Notifications";
  if (unread > 0 || needsDecision > 0) label += `, ${unread} unread`;
  if (needsDecision > 0)
    label += `, ${needsDecision} ${needsDecision === 1 ? "needs" : "need"} your decision`;
  return label;
}

export const NOTIFICATION_BADGE_CAP = 99;
/** Bell badge text: empty when nothing is unread, capped at "99+". */
export function notificationBadge(unread: number): string {
  if (!Number.isFinite(unread) || unread <= 0) return "";
  return unread > NOTIFICATION_BADGE_CAP
    ? `${NOTIFICATION_BADGE_CAP}+`
    : String(Math.floor(unread));
}

/** Coalesced rows say how many updates arrived since the row was opened. */
export function notificationCountLabel(count: number): string | null {
  return Number.isFinite(count) && count > 1
    ? `${Math.floor(count)} new`
    : null;
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
  const parts = [
    item.actor ? notificationSentence(item).text : notificationTitle(item),
  ];
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
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
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
  for (const value of typeof frontendUrls === "string"
    ? [frontendUrls]
    : frontendUrls) {
    try {
      const origin = new URL(value).origin;
      if (
        /^https?:$/.test(new URL(value).protocol) &&
        !origins.includes(origin)
      )
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
  if (
    link.kind === "chat" &&
    !link.agentId &&
    refs.kind === "chat" &&
    refs.agentId
  )
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
    if (entry.conversation_id)
      add(conversations, entry.conversation_id, entry.unread);
    if (
      entry.source_type === "conversation" &&
      entry.id !== entry.conversation_id
    )
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
  const payload = data as {
    notification_id?: unknown;
    notificationId?: unknown;
  };
  return (
    asNonEmptyString(payload.notification_id) ??
    asNonEmptyString(payload.notificationId)
  );
}

/** One client-owned action vocabulary for rows, accessibility and tests. */
export function notificationSentence(item: AssistantNotification) {
  const actor =
    item.actor?.display_name?.trim() ||
    item.actor?.name ||
    item.title ||
    "NyxID";
  const target = item.source.label?.trim() || "";
  let action: string;
  let showTarget = true;
  switch (item.kind) {
    case "automation_result":
      action = "finished";
      showTarget = false;
      break;
    case "automation_problem":
      action =
        item.outcome === "paused"
          ? "was paused"
          : item.outcome === "delivery_failed" ||
              item.reason === "result_delivery_failed"
            ? "could not deliver its result"
            : item.outcome === "skipped"
              ? "was skipped"
              : "failed";
      showTarget = false;
      break;
    case "needs_decision":
      action = item.resolved
        ? "closed a decision in"
        : "needs your decision in";
      break;
    case "agent_update":
      action =
        item.outcome === "failed"
          ? "could not finish in"
          : item.actor?.kind === "specialist" && !item.source.group_id
            ? "reported back in"
            : "replied in";
      break;
    case "chat_update":
      action = "replied in";
      break;
    case "group_mention":
      action = "mentioned you in";
      break;
  }
  // Old servers have no actor field. Preserve their complete title during rollout.
  if (!item.actor) {
    const title = notificationTitle(item);
    return { actor: title, action: "", target: "", platform: "", text: title };
  }
  const platform =
    item.kind === "chat_update" && item.platform
      ? notificationPlatformLabel(item.platform)
      : "";
  const shownTarget = showTarget ? target : "";
  if (!shownTarget && item.kind === "needs_decision")
    action = item.resolved ? "closed a decision" : "needs your decision";
  const suffix = platform ? `on ${platform}` : "";
  return {
    actor,
    action,
    target: shownTarget,
    platform: suffix,
    text: [actor, action, shownTarget, suffix].filter(Boolean).join(" "),
  };
}

export function notificationPlatformLabel(platform: string): string {
  const names: Record<string, string> = {
    lark: "Lark",
    telegram: "Telegram",
    discord: "Discord",
    slack: "Slack",
    whatsapp: "WhatsApp",
    feishu: "Feishu",
    wecom: "WeCom",
    dingtalk: "DingTalk",
    teams: "Teams",
  };
  return names[platform] ?? platform;
}

/** Extra source context, never a second copy of the sentence target. */
export function notificationContextLabel(item: AssistantNotification): string | null {
  const target = notificationSentence(item).target;
  const agent = item.actor?.type === "agent" ? item.actor : undefined;
  const agentName = agent?.display_name?.trim() || agent?.name.trim() || "";
  let context = item.source.label?.trim() || "";
  if (item.kind === "chat_update") {
    // Channel titles already contain the live bot label followed by the exact chat label.
    const suffix = context ? ` · ${context}` : "";
    context = item.source.channel_id && suffix && item.title.endsWith(suffix)
      ? item.title.slice(0, -suffix.length).trim()
      : agentName;
  } else if (item.kind === "automation_result" || item.kind === "automation_problem") {
    // An automation identity names the trigger, not the agent that ran it.
    context = agentName;
  }
  return context && context !== target ? context : null;
}

export function notificationFilterCount(
  summary: NotificationSummary | undefined,
  filter: NotificationFilter,
): number {
  if (filter === "all") return summary?.unread_count ?? 0;
  if (filter === "needs_you") return summary?.needs_decision_count ?? 0;
  const kinds: NotificationKind[] =
    filter === "automations"
      ? ["automation_result", "automation_problem"]
      : ["agent_update", "chat_update", "group_mention"];
  return kinds.reduce(
    (total, kind) => total + (summary?.by_kind?.[kind] ?? 0),
    0,
  );
}
