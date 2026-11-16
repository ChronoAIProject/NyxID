import { translateUi } from "@/i18n/use-ui-translation";
import {
  ACCOUNT_SETTINGS_LABELS,
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

/** Use the current locale and the same labels that each editor renders. */
export function accountSettingsSearchTerms(section: AccountSettingsSection): string[] {
  return [
    section,
    ...[ACCOUNT_SETTINGS_TITLES[section], ...(ACCOUNT_SETTINGS_LABELS[section] ?? [])].map(
      (label) => translateUi(label),
    ),
  ];
}
export function billingSearchTerms(section: BillingSection): string[] {
  const labels = [BILLING_SECTION_TITLES[section], ...(BILLING_SECTION_LABELS[section] ?? [])].map(
    (label) => translateUi(label),
  );
  if (section === "usage/explorer" || section === "usage/charts/quantities")
    labels.push(...Object.values(dimensions), ...BILLING_METRICS.map(billingMetricLabel));
  return [section, ...labels];
}
export function agentDetailsSearchTerms(
  section: AgentDetailsSection,
  agent: AssistantAgent,
  advancedControls = false,
): string[] {
  const labels: string[] = [
    AGENT_DETAILS_TITLES[section],
    ...(AGENT_DETAILS_LABELS[section] ?? []),
  ];
  if (section in AGENT_PROFILE_FIELDS)
    labels.push(
      ...Object.entries(AGENT_PROFILE_FIELDS[section as keyof typeof AGENT_PROFILE_FIELDS])
        .filter(
          ([key]) =>
            !(key === "name" && agent.kind === "nyxbot") && (key !== "model" || advancedControls),
        )
        .map(([, label]) => label),
    );
  if (section === "grants/services" && agent.owner_kind === "org")
    labels.splice(labels.indexOf("Read my account"), 1);
  if (section === "overview/persona" && agent.kind === "nyxbot") labels.push(AGENT_NOTES_LABEL);
  if (section === "overview/access")
    labels.push("Workspace setting", ...Object.values(ACCESS_MODE_LABEL));
  return [section, ...labels.map((label) => translateUi(label))];
}
