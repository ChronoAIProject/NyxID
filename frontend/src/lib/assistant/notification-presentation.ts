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
  NotificationPushPreferences,
} from "@/schemas/assistant-notifications";
import { translateUi } from "@/i18n/use-ui-translation";

/** Keep exact counts through 99, then bound the bell's compact pill. */
export const NOTIFICATION_BADGE_CAP = 99;

export function notificationBadgeText(unread: number): string {
  return unread > NOTIFICATION_BADGE_CAP ? `${String(NOTIFICATION_BADGE_CAP)}+` : String(unread);
}

export function notificationBellLabel(unread: number, needsDecision: number): string {
  if (unread <= 0 && needsDecision <= 0) return translateUi("Notifications");
  const parts = [translateUi("{{count}} unread", { count: unread })];
  if (needsDecision > 0) {
    parts.push(needsDecision === 1
      ? translateUi("{{count}} needs you", { count: needsDecision })
      : translateUi("{{count}} need you", { count: needsDecision }));
  }
  return translateUi("Notifications, {{status}}", { status: parts.join(", ") });
}

export interface NotificationKindPresentation {
  readonly icon: LucideIcon;
  readonly label: string;
  readonly tone: string;
}

/** Semantic colour does the status work; neutral kinds stay quiet. */
export const NOTIFICATION_KIND_PRESENTATION: Record<NotificationKind, NotificationKindPresentation> = {
  automation_result: { icon: CircleCheck, label: "Automation finished", tone: "text-success" },
  automation_problem: {
    icon: AlertTriangle,
    label: "Automation needs attention",
    tone: "text-destructive",
  },
  needs_decision: { icon: CircleHelp, label: "Needs your decision", tone: "text-warning" },
  agent_update: { icon: Bot, label: "Agent update", tone: "text-text-tertiary" },
  chat_update: { icon: MessageSquare, label: "Chat update", tone: "text-text-tertiary" },
  group_mention: { icon: AtSign, label: "Mentioned you", tone: "text-text-tertiary" },
};

/** The accessible name of a feed row: title, then unread and pending-decision state. */
export function notificationRowLabel(item: AssistantNotification): string {
  const kind = NOTIFICATION_KIND_PRESENTATION[item.kind];
  const parts = [item.title || translateUi(kind.label)];
  if (!item.read) parts.push(translateUi("unread"));
  if (item.kind === "needs_decision" && !item.resolved) parts.push(translateUi("needs your decision"));
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
    description: "A run failed, was skipped, could not deliver its result or paused the automation.",
  },
  {
    name: "needs_decision",
    label: "Decisions waiting for you",
    description: "An agent is waiting on one of your action cards.",
  },
  {
    name: "agent_update",
    label: "Agent updates",
    description: "A specialist reported back or an agent replied in the background.",
  },
  {
    name: "chat_update",
    label: "Chat updates",
    description: "Replies in your Telegram, Lark, Discord and other channel chats.",
  },
  {
    name: "group_mention",
    label: "Group mentions",
    description: "Someone mentioned you in a collaboration group.",
  },
];
