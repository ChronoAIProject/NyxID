import { sectionNavigation } from "@/lib/section-navigation";
/** URL vocabulary shared by navigation, search and the backend drift guard. */
export const NYXBOT_SETTINGS_SECTIONS = [
  "permissions",
  "permissions/specialists",
  "tasks",
  "tasks/specialists",
  "tasks/groups",
  "automations",
  "automations/budgets",
  "notifications",
  "channels",
  "channels/adapters",
  "channels/links",
] as const;
export type NyxBotSettingsSection = (typeof NYXBOT_SETTINGS_SECTIONS)[number];
export const NYXBOT_SETTINGS_TITLES: Record<NyxBotSettingsSection, string> = {
  permissions: "Permissions",
  "permissions/specialists": "Specialist access requests",
  tasks: "Tasks & specialists",
  "tasks/specialists": "Specialists",
  "tasks/groups": "Group hand-offs",
  automations: "Automations",
  "automations/budgets": "Timezone and budgets",
  notifications: "Notifications",
  channels: "Chats & channels",
  "channels/adapters": "Adapter bindings",
  "channels/links": "Personal links",
};
const navigation = sectionNavigation(NYXBOT_SETTINGS_SECTIONS);
export const parseNyxBotSettingsSection = navigation.parse;
export const nyxBotSettingsParent = navigation.parent;
export const nyxBotSettingsAncestry = navigation.ancestry;
export const availableNyxBotSettingsSection = navigation.available;

export const NYXBOT_SETTINGS_KEYWORDS: Record<NyxBotSettingsSection, readonly string[]> = {
  permissions: ["access mode", "Auto", "Full access", "approvals"],
  "permissions/specialists": ["requests", "deny", "route", "grants"],
  tasks: ["continuations", "limits", "resume"],
  "tasks/specialists": ["live specialists", "concurrent", "parallel", "limits"],
  "tasks/groups": ["handoffs", "per message", "per hour", "limits"],
  automations: ["schedules", "webhooks", "recurring"],
  "automations/budgets": ["timezone", "budgets", "runs", "interval"],
  notifications: ["push", "phone", "kinds"],
  channels: ["bots", "Telegram", "Discord", "Slack", "organisation"],
  "channels/adapters": ["phone", "speech", "bindings"],
  "channels/links": ["personal", "platform", "link", "account"],
};

export function nyxBotSettingsHref(section?: NyxBotSettingsSection): string {
  const search = new URLSearchParams({ panel: "nyxbot" });
  if (section) search.set("panelTab", section);
  return `/assistant?${search.toString()}`;
}
