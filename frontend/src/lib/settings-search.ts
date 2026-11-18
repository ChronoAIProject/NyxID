import { translateUi } from "@/i18n/use-ui-translation";
import {
  ACCOUNT_SETTINGS_LABELS,
  ACCOUNT_FIELD_LABELS,
  ACCOUNT_SETTINGS_SECTIONS,
  accountSettingsPageTitle,
  ACCOUNT_SETTINGS_TITLES,
  type AccountSettingsSection,
} from "./account-settings-sections";
import {
  BILLING_SECTION_LABELS,
  BILLING_SECTION_TITLES,
  type BillingSection,
} from "./billing-sections";
import {
  AGENT_DETAILS_LABELS,
  AGENT_DETAILS_TITLES,
  AGENT_NOTES_LABEL,
  AGENT_PROFILE_FIELDS,
  type AgentDetailsSection,
} from "./assistant/agent-details-sections";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { dimensions } from "./billing-usage";
import { BILLING_METRICS } from "@/schemas/billing-metrics";
import { billingMetricLabel } from "./billing-units";
import { ACCESS_MODE_LABEL } from "./assistant/nyxbot-labels";

const PASSWORD_LABELS = new Set<string>([
  ACCOUNT_FIELD_LABELS.currentPassword,
  ACCOUNT_FIELD_LABELS.newPassword,
  ACCOUNT_FIELD_LABELS.confirmNewPassword,
]);

/** Use the current locale and the same labels that each editor renders. */
export function accountSettingsSearchEntries(hasPassword = true) {
  return ACCOUNT_SETTINGS_SECTIONS.flatMap(section =>
    [...new Set([ACCOUNT_SETTINGS_TITLES[section], ...(ACCOUNT_SETTINGS_LABELS[section] ?? [])])]
      .filter(label => hasPassword || !PASSWORD_LABELS.has(label))
      .map((label, index) => ({
        id: `${section}:${index}`,
        section,
        setting: label,
        label: translateUi(label),
        hint: translateUi(accountSettingsPageTitle(section.split("/")[0] as AccountSettingsSection)),
      })),
  );
}

export function accountSettingsSearchTerms(section: AccountSettingsSection): string[] {
  return [section, ...accountSettingsSearchEntries().filter(entry => entry.section === section).map(entry => entry.label)];
}
export function billingSearchTerms(section: BillingSection): string[] {
  const labels = [BILLING_SECTION_TITLES[section], ...(BILLING_SECTION_LABELS[section] ?? [])].map(
    (label) => translateUi(label),
  );
  if (section === "usage/explorer" || section === "usage/charts")
    labels.push(...Object.values(dimensions), ...BILLING_METRICS.map(billingMetricLabel));
  return [section, ...labels];
}
export function agentDetailsSearchTerms(
  section: AgentDetailsSection,
  agent: AssistantAgent,
  advancedControls = false,
  features: { voice?: boolean; learning?: boolean; orgChannels?: boolean; owner?: string } = {},
): string[] {
  const labels: string[] = [
    AGENT_DETAILS_TITLES[section],
    ...(AGENT_DETAILS_LABELS[section] ?? []).filter(label => {
      if (label === "Device voice") return features.voice !== false && features.owner !== "" && !agent.share_role && agent.owner_kind !== "org" && agent.status !== "destroyed";
      if (["Automatic learning", "Threshold", "Learning proposals"].includes(label)) return features.learning !== false && !agent.share_role;
      if (["Operation limits", "Guest access", "Guest operation limits"].includes(label)) return !agent.share_role && agent.kind === "specialist" && agent.can_maintain !== false && (label === "Operation limits" || agent.owner_kind !== "org" || (features.orgChannels && agent.org_role === "admin"));
      return true;
    }),
  ];
  if (section in AGENT_PROFILE_FIELDS)
    labels.push(
      ...Object.entries(AGENT_PROFILE_FIELDS[section as keyof typeof AGENT_PROFILE_FIELDS])
        .filter(
          ([key]) =>
            !(key === "name" && agent.kind === "nyxbot") && (key !== "model" || advancedControls) && (key !== "description" || (agent.kind === "nyxbot" ? section === "overview/persona" : section === "overview")),
        )
        .map(([, label]) => label),
    );
  if (section === "grants/services" && agent.owner_kind === "org")
    labels.splice(labels.indexOf("Read my account"), 1);
  if (section === "overview/persona" && agent.kind === "nyxbot") labels.push(AGENT_NOTES_LABEL);
  if (section === "overview")
    labels.push("Workspace setting", ...Object.values(ACCESS_MODE_LABEL));
  return [section, ...labels.map((label) => translateUi(label))];
}
