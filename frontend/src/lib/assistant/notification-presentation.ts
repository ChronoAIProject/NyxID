import {
  AlertTriangle,
  AtSign,
  Bot,
  CircleCheck,
  CircleHelp,
  MessageSquare,
  type LucideIcon,
} from "lucide-react";
import type {
  AssistantNotification,
  NotificationKind,
  NotificationSummary,
  NotificationFilter,
  NotificationPushPreferences,
} from "@/schemas/assistant-notifications";
import { translateUi } from "@/i18n/use-ui-translation";

/** Keep exact counts through 99, then bound the bell's compact pill. */
export const NOTIFICATION_BADGE_CAP = 99;

export function notificationBadgeText(unread: number): string {
  return unread > NOTIFICATION_BADGE_CAP
    ? `${String(NOTIFICATION_BADGE_CAP)}+`
    : String(unread);
}

export function notificationBellLabel(
  unread: number,
  needsDecision: number,
): string {
  if (unread <= 0 && needsDecision <= 0) return translateUi("Notifications");
  const parts = [translateUi("{{count}} unread", { count: unread })];
  if (needsDecision > 0) {
    parts.push(
      needsDecision === 1
        ? translateUi("{{count}} needs you", { count: needsDecision })
        : translateUi("{{count}} need you", { count: needsDecision }),
    );
  }
  return translateUi("Notifications, {{status}}", { status: parts.join(", ") });
}

export interface NotificationKindPresentation {
  readonly icon: LucideIcon;
  readonly label: string;
  readonly tone: string;
}

/** Semantic colour does the status work; neutral kinds stay quiet. */
export const NOTIFICATION_KIND_PRESENTATION: Record<
  NotificationKind,
  NotificationKindPresentation
> = {
  automation_result: {
    icon: CircleCheck,
    label: "Automation finished",
    tone: "text-success",
  },
  automation_problem: {
    icon: AlertTriangle,
    label: "Automation needs attention",
    tone: "text-destructive",
  },
  needs_decision: {
    icon: CircleHelp,
    label: "Needs your decision",
    tone: "text-warning",
  },
  agent_update: {
    icon: Bot,
    label: "Agent update",
    tone: "text-text-tertiary",
  },
  chat_update: {
    icon: MessageSquare,
    label: "Chat update",
    tone: "text-text-tertiary",
  },
  group_mention: {
    icon: AtSign,
    label: "Mentioned you",
    tone: "text-text-tertiary",
  },
};

/** The accessible name of a feed row: title, then unread and pending-decision state. */
export function notificationRowLabel(item: AssistantNotification): string {
  const kind = NOTIFICATION_KIND_PRESENTATION[item.kind];
  const parts = [
    item.actor
      ? notificationSentence(item).text
      : item.title || translateUi(kind.label),
  ];
  if (!item.read) parts.push(translateUi("unread"));
  if (item.kind === "needs_decision" && !item.resolved)
    parts.push(translateUi("needs your decision"));
  return parts.join(", ");
}

/** One push switch per kind in NyxBot settings, in display order. */
export const NOTIFICATION_PUSH_ROWS: readonly {
  readonly name: keyof NotificationPushPreferences;
  readonly label: string;
  readonly description: string;
}[] = [
  {
    name: "automation_result",
    label: "Automation results",
    description: "A scheduled or webhook automation finished its task.",
  },
  {
    name: "automation_problem",
    label: "Automation problems",
    description:
      "A run failed, was skipped, could not deliver its result or paused the automation.",
  },
  {
    name: "needs_decision",
    label: "Decisions waiting for you",
    description: "An agent is waiting on one of your action cards.",
  },
  {
    name: "agent_update",
    label: "Agent updates",
    description:
      "A specialist reported back or an agent replied in the background.",
  },
  {
    name: "chat_update",
    label: "Chat updates",
    description:
      "Replies in your Telegram, Lark, Discord and other channel chats.",
  },
  {
    name: "group_mention",
    label: "Group mentions",
    description: "Someone mentioned you in a collaboration group.",
  },
];

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
      action = translateUi("finished");
      showTarget = false;
      break;
    case "automation_problem":
      action =
        item.outcome === "paused"
          ? translateUi("was paused")
          : item.outcome === "delivery_failed" ||
              item.reason === "result_delivery_failed"
            ? translateUi("could not deliver its result")
            : item.outcome === "skipped"
              ? translateUi("was skipped")
              : translateUi("failed");
      showTarget = false;
      break;
    case "needs_decision":
      action = item.resolved
        ? translateUi("closed a decision in")
        : translateUi("needs your decision in");
      break;
    case "agent_update":
      action =
        item.outcome === "failed"
          ? translateUi("could not finish in")
          : item.actor?.kind === "specialist" && !item.source.group_id
            ? translateUi("reported back in")
            : translateUi("replied in");
      break;
    case "chat_update":
      action = translateUi("replied in");
      break;
    case "group_mention":
      action = translateUi("mentioned you in");
      break;
  }
  // Old servers have no actor field. Preserve their complete title during rollout.
  if (!item.actor) {
    const title =
      item.title ||
      target ||
      translateUi(NOTIFICATION_KIND_PRESENTATION[item.kind].label);
    return { actor: title, action: "", target: "", platform: "", text: title };
  }
  const platform =
    item.kind === "chat_update" && item.platform
      ? notificationPlatformLabel(item.platform)
      : "";
  const shownTarget = showTarget ? target : "";
  if (!shownTarget && item.kind === "needs_decision")
    action = item.resolved
      ? translateUi("closed a decision")
      : translateUi("needs your decision");
  const suffix = platform ? translateUi("on {{platform}}", { platform }) : "";
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
export function notificationContextLabel(
  item: AssistantNotification,
): string | null {
  const target = notificationSentence(item).target;
  const agent = item.actor?.type === "agent" ? item.actor : undefined;
  const agentName = agent?.display_name?.trim() || agent?.name.trim() || "";
  let context = item.source.label?.trim() || "";
  if (item.kind === "chat_update") {
    // Channel titles already contain the live bot label followed by the exact chat label.
    const suffix = context ? ` · ${context}` : "";
    context =
      item.source.channel_id && suffix && item.title.endsWith(suffix)
        ? item.title.slice(0, -suffix.length).trim()
        : agentName;
  } else if (
    item.kind === "automation_result" ||
    item.kind === "automation_problem"
  ) {
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
