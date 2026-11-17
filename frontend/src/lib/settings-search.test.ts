import "@/i18n";
import { afterEach, expect, it } from "vitest";
import { appI18n } from "@/i18n/instance";
import { translateUi } from "@/i18n/use-ui-translation";
import zhCN from "@/i18n/locales/zh-CN.json";
import { ACCOUNT_SETTINGS_LABELS, type AccountSettingsSection } from "./account-settings-sections";
import { BILLING_SECTION_LABELS, type BillingSection } from "./billing-sections";
import { AGENT_PROFILE_FIELDS, type AgentDetailsSection } from "./assistant/agent-details-sections";
import { assistantAgentSchema } from "@/schemas/assistant-nyxagent";
import {
  accountSettingsSearchTerms,
  agentDetailsSearchTerms,
  billingSearchTerms,
} from "./settings-search";

afterEach(async () => {
  await appI18n.changeLanguage("en");
});
it.each(["en", "zh-CN"])(
  "indexes each rendered field label in the current language (%s)",
  async (language) => {
    appI18n.addResourceBundle("zh-CN", "app", zhCN, true, true);
    await appI18n.changeLanguage(language);
    for (const [section, labels] of Object.entries(ACCOUNT_SETTINGS_LABELS))
      for (const label of labels)
        expect(accountSettingsSearchTerms(section as AccountSettingsSection)).toContain(
          translateUi(label),
        );
    for (const [section, labels] of Object.entries(BILLING_SECTION_LABELS))
      for (const label of labels)
        expect(billingSearchTerms(section as BillingSection)).toContain(translateUi(label));
    const agent = assistantAgentSchema.parse({
      id: "a",
      name: "Scout",
      kind: "specialist",
      status: "idle",
      created_at: "now",
      last_active_at: "now",
    });
    for (const [section, fields] of Object.entries(AGENT_PROFILE_FIELDS))
      for (const [key, label] of Object.entries(fields)) {
        if (section === "overview/persona" && key === "description") continue;
        expect(agentDetailsSearchTerms(section as AgentDetailsSection, agent, true)).toContain(
          translateUi(label),
        );
      }
    expect(agentDetailsSearchTerms("overview/persona", { ...agent, kind: "nyxbot" })).not.toContain(
      translateUi("Name"),
    );
    expect(
      agentDetailsSearchTerms("grants/services", { ...agent, owner_kind: "org" }),
    ).not.toContain(translateUi("Read my account"));
  },
);
