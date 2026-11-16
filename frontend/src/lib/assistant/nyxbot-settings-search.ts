import { translateUi } from "@/i18n/use-ui-translation";
import { ACCESS_MODE_LABEL } from "./nyxbot-labels";
import { NOTIFICATION_PUSH_ROWS } from "./notification-presentation";
import {
  AUTOMATION_BUDGET_ROWS,
  AUTOMATION_TIMEZONE_LABEL,
  SETTINGS_LABELS,
  SETTINGS_PAGE_FIELDS,
  SPECIALIST_ACCESS_LABELS,
} from "./nyxbot-settings-fields";
import { NYXBOT_SETTINGS_KEYWORDS, NYXBOT_SETTINGS_TITLES, type NyxBotSettingsSection } from "./nyxbot-settings-sections";

/** Evaluate at render time so search uses the same current-language labels as forms. */
export function nyxBotSettingsSearchTerms(section: NyxBotSettingsSection): string[] {
  const labels = (SETTINGS_PAGE_FIELDS[section] ?? []).flatMap(field => {
    if (field === "access_mode") return ["Access mode", ...Object.values(ACCESS_MODE_LABEL)];
    if (field === "specialist_access_requests") return [NYXBOT_SETTINGS_TITLES[section], ...Object.values(SPECIALIST_ACCESS_LABELS)];
    return [SETTINGS_LABELS[field]];
  });
  if (section === "automations/budgets") labels.push(AUTOMATION_TIMEZONE_LABEL, ...AUTOMATION_BUDGET_ROWS.map(row => row[1]));
  if (section === "notifications") labels.push(...NOTIFICATION_PUSH_ROWS.map(row => row.label));
  return [...NYXBOT_SETTINGS_KEYWORDS[section], ...labels.map(label => translateUi(label))];
}
