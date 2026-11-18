import { sectionNavigation } from "@/lib/section-navigation";
/** URL vocabulary shared by navigation, search and the backend drift guard. */
export const NYXBOT_SETTINGS_SECTIONS = [
  "permissions",
  "tasks",
  "automations",
  "notifications",
  "channels",
  "channels/adapters",
  "channels/links",
] as const;
export type NyxBotSettingsSection = (typeof NYXBOT_SETTINGS_SECTIONS)[number];
export const NYXBOT_SETTINGS_TITLES: Record<NyxBotSettingsSection, string> = {
  permissions: "Permissions",
  tasks: "Tasks & specialists",
  automations: "Automations",
  notifications: "Notifications",
  channels: "Chats & channels",
  "channels/adapters": "Adapter bindings",
  "channels/links": "Personal links",
};
const navigation = sectionNavigation(NYXBOT_SETTINGS_SECTIONS);
export const parseNyxBotSettingsSection = navigation.normalize;
export const nyxBotSettingsParent = navigation.parent;
export const nyxBotSettingsAncestry = navigation.ancestry;
export const availableNyxBotSettingsSection = navigation.available;

export const NYXBOT_SETTINGS_KEYWORDS: Record<NyxBotSettingsSection, readonly string[]> = {
  permissions: ["access mode", "Auto", "Full access", "approvals", "requests", "deny", "route", "grants"],
  tasks: ["continuations", "limits", "resume", "live specialists", "concurrent", "parallel", "handoffs", "per message", "per hour"],
  automations: ["schedules", "webhooks", "recurring", "timezone", "budgets", "runs", "interval"],
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
