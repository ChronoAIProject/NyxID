import "@/i18n";
import { afterEach, expect, it } from "vitest";
import { appI18n } from "@/i18n/instance";
import { translateUi } from "@/i18n/use-ui-translation";
import zhCN from "@/i18n/locales/app/zh-CN";
import zhTW from "@/i18n/locales/app/zh-TW";
import {
  ACCOUNT_FIELD_LABELS,
  ACCOUNT_SETTINGS_LABELS,
  ACCOUNT_SETTINGS_SECTIONS,
  ACCOUNT_SETTINGS_TITLES,
} from "./account-settings-sections";
import {
  BILLING_SECTION_LABELS,
  BILLING_SECTIONS,
  BILLING_SECTION_TITLES,
} from "./billing-sections";
import {
  AGENT_DETAILS_LABELS,
  AGENT_DETAILS_SECTIONS,
  AGENT_DETAILS_TITLES,
  AGENT_NOTES_LABEL,
  AGENT_PROFILE_FIELDS,
  type AgentDetailsSection,
  agentDetailsAvailability,
  agentDetailsSections,
} from "./assistant/agent-details-sections";
import { assistantAgentSchema } from "@/schemas/assistant-nyxagent";
import { ACCESS_MODE_LABEL } from "./assistant/nyxbot-labels";
import {
  AUTOMATION_BUDGET_ROWS,
  AUTOMATION_TIMEZONE_LABEL,
  SETTINGS_LABELS,
  SPECIALIST_ACCESS_LABELS,
} from "./assistant/nyxbot-settings-fields";
import { NOTIFICATION_PUSH_ROWS } from "./assistant/notification-presentation";
import {
  NYXBOT_SETTINGS_SECTIONS,
  NYXBOT_SETTINGS_TITLES,
  availableNyxBotSettingsSection,
} from "./assistant/nyxbot-settings-sections";
import { nyxBotSettingsSearchTerms } from "./assistant/nyxbot-settings-search";
import { dimensions } from "./billing-usage";
import { BILLING_METRICS } from "@/schemas/billing-metrics";
import { billingMetricLabel } from "./billing-units";
import {
  accountSettingsSearchEntries,
  accountSettingsSearchTerms,
  agentDetailsSearchTerms,
  billingSearchTerms,
} from "./settings-search";

const specialist = assistantAgentSchema.parse({
  id: "a",
  name: "Scout",
  kind: "specialist",
  status: "idle",
  created_at: "now",
  last_active_at: "now",
});
const enabled = {
  voice: true,
  learning: true,
  orgChannels: true,
  owner: "person",
};

// These are the visible section result's label/hint and keywords, as consumed by
// workspace search. No React tree or interaction is needed to check every field.
function entries<S extends string>(
  sections: readonly S[],
  titles: Record<S, string>,
  hint: string,
  terms: (section: S) => string[],
) {
  return sections.map((section) => ({
    section,
    label: translateUi(titles[section]),
    hint: translateUi(hint),
    terms: terms(section),
  }));
}
function matches(
  entry: { label: string; hint: string; terms: string[] },
  label: string,
) {
  const haystack = [entry.label, entry.hint, ...entry.terms]
    .join(" ")
    .toLowerCase();
  return label
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => haystack.includes(word));
}
function uniqueResults(results: readonly { label: string; hint: string }[]) {
  const pairs = results.map((entry) =>
    JSON.stringify([entry.label, entry.hint]),
  );
  expect(pairs.filter((pair, index) => pairs.indexOf(pair) !== index)).toEqual(
    [],
  );
}
function labelsResolve<S extends string>(
  results: ReturnType<typeof entries<S>>,
  expected: Partial<Record<S, readonly string[]>>,
  rendered: Partial<Record<S, readonly string[]>> = {},
) {
  uniqueResults(results);
  const sectionsByLabel = new Map<string, Set<S>>();
  for (const entry of results) {
    sectionsByLabel.set(
      entry.label,
      new Set([...(sectionsByLabel.get(entry.label) ?? []), entry.section]),
    );
    const labels = [
      ...(expected[entry.section] ?? []).map((label) => translateUi(label)),
      ...(rendered[entry.section] ?? []),
    ];
    for (const label of labels) {
      expect(entry.terms, `${entry.section}: ${label}`).toContain(label);
      expect(matches(entry, label), `${entry.section}: ${label}`).toBe(true);
      sectionsByLabel.set(
        label,
        new Set([...(sectionsByLabel.get(label) ?? []), entry.section]),
      );
    }
  }
  for (const [label, sections] of sectionsByLabel)
    expect(
      results
        .filter((entry) => entry.label === label || entry.terms.includes(label))
        .map((entry) => entry.section)
        .sort(),
      label,
    ).toEqual([...sections].sort());
}

afterEach(async () => {
  await appI18n.changeLanguage("en");
});
for (const language of ["en", "zh-CN", "zh-TW"]) {
  async function locale() {
    appI18n.addResourceBundle("zh-CN", "app", zhCN, true, true);
    appI18n.addResourceBundle("zh-TW", "app", zhTW, true, true);
    await appI18n.changeLanguage(language);
  }
  it(`maps every Account workspace and sidebar setting in ${language}`, async () => {
    await locale();
    const workspace = entries(
      ACCOUNT_SETTINGS_SECTIONS,
      ACCOUNT_SETTINGS_TITLES,
      "Account Settings",
      accountSettingsSearchTerms,
    );
    labelsResolve(workspace, ACCOUNT_SETTINGS_LABELS);
    const sidebar = accountSettingsSearchEntries();
    uniqueResults(sidebar);
    for (const [section, labels] of Object.entries(ACCOUNT_SETTINGS_LABELS))
      for (const label of labels) {
        expect(
          sidebar
            .filter((entry) => entry.label === translateUi(label))
            .map((entry) => entry.section),
        ).toEqual([section]);
        expect(sidebar.find((entry) => entry.setting === label)?.section).toBe(
          section,
        );
      }
    const unavailable = accountSettingsSearchEntries(false);
    for (const label of [
      ACCOUNT_FIELD_LABELS.currentPassword,
      ACCOUNT_FIELD_LABELS.newPassword,
      ACCOUNT_FIELD_LABELS.confirmNewPassword,
    ])
      expect(unavailable.map((entry) => entry.label)).not.toContain(
        translateUi(label),
      );
    expect(
      unavailable
        .filter((entry) => entry.section === "security")
        .map((entry) => entry.setting),
    ).toEqual([
      "Security",
      ACCOUNT_FIELD_LABELS.twofactorAuthentication,
      ACCOUNT_FIELD_LABELS.skipMfaForApprovedSignins,
      ACCOUNT_FIELD_LABELS.linkedAccounts,
    ]);
    // Each displayed sidebar result has a single section, including category titles.
    for (const section of ACCOUNT_SETTINGS_SECTIONS)
      expect(
        sidebar.some(
          (entry) =>
            entry.section === section &&
            entry.setting === ACCOUNT_SETTINGS_TITLES[section],
        ),
      ).toBe(true);
  });
  it(`maps every NyxBot field and option in ${language} and omits unavailable pages`, async () => {
    await locale();
    const results = entries(
      NYXBOT_SETTINGS_SECTIONS,
      NYXBOT_SETTINGS_TITLES,
      "NyxBot settings",
      nyxBotSettingsSearchTerms,
    );
    labelsResolve(results, {
      permissions: [
        "Access mode",
        ...Object.values(ACCESS_MODE_LABEL),
        "Specialist access requests",
        ...Object.values(SPECIALIST_ACCESS_LABELS),
      ],
      tasks: Object.values(SETTINGS_LABELS),
      automations: [
        AUTOMATION_TIMEZONE_LABEL,
        ...AUTOMATION_BUDGET_ROWS.map((row) => row[1]),
      ],
      notifications: NOTIFICATION_PUSH_ROWS.map((row) => row.label),
    });
    for (const notifications of [false, true])
      for (const adapters of [false, true]) {
        const available = results.filter(
          (entry) =>
            availableNyxBotSettingsSection(entry.section, {
              notifications,
              "channels/adapters": adapters,
            }) === entry.section,
        );
        expect(available.map((entry) => entry.section)).toEqual(
          NYXBOT_SETTINGS_SECTIONS.filter(
            (section) =>
              (notifications || section !== "notifications") &&
              (adapters || section !== "channels/adapters"),
          ),
        );
        if (!notifications)
          for (const row of NOTIFICATION_PUSH_ROWS)
            expect(
              available.some((entry) =>
                entry.terms.includes(translateUi(row.label)),
              ),
            ).toBe(false);
      }
  });
  it(`maps every Billing field, dimension and metric in ${language}`, async () => {
    await locale();
    const results = entries(
      BILLING_SECTIONS,
      BILLING_SECTION_TITLES,
      "Billing & Usage",
      billingSearchTerms,
    );
    const quantities = [
      ...Object.values(dimensions),
      ...BILLING_METRICS.map(billingMetricLabel),
    ];
    labelsResolve(results, BILLING_SECTION_LABELS, {
      "usage/charts": quantities,
      "usage/explorer": quantities,
    });
    for (const section of ["usage/charts", "usage/explorer"]) {
      const entry = results.find((candidate) => candidate.section === section)!;
      for (const label of [
        ...Object.values(dimensions),
        ...BILLING_METRICS.map(billingMetricLabel),
      ])
        expect(entry.terms).toContain(label);
    }
  });
  it(`maps every rendered agent field in ${language} across authority and feature variants`, async () => {
    await locale();
    for (const agent of [
      specialist,
      { ...specialist, kind: "nyxbot" as const },
      { ...specialist, owner_kind: "org" as const, org_role: "admin" as const },
      {
        ...specialist,
        owner_kind: "org" as const,
        org_role: "member" as const,
      },
      { ...specialist, can_maintain: false },
      { ...specialist, share_role: "user" as const },
      { ...specialist, share_role: "manager" as const },
      { ...specialist, status: "destroyed" as const },
    ]) {
      const available = agentDetailsAvailability(agent, enabled);
      const results = entries(
        AGENT_DETAILS_SECTIONS.filter(
          (section) =>
            agentDetailsSections.available(section, available) === section,
        ),
        AGENT_DETAILS_TITLES,
        "Scout",
        (section) => agentDetailsSearchTerms(section, agent, true, enabled),
      );
      const expected: Partial<Record<AgentDetailsSection, string[]>> = {};
      for (const entry of results) {
        const labels: string[] = [];
        expected[entry.section] = labels;
        for (const label of AGENT_DETAILS_LABELS[entry.section] ?? []) {
          if (
            label === "Device voice" &&
            (agent.share_role ||
              agent.owner_kind === "org" ||
              agent.status === "destroyed")
          )
            continue;
          if (
            ["Automatic learning", "Threshold", "Learning proposals"].includes(
              label,
            ) &&
            agent.share_role
          )
            continue;
          if (
            [
              "Operation limits",
              "Guest access",
              "Guest operation limits",
            ].includes(label) &&
            (agent.share_role ||
              agent.kind !== "specialist" ||
              agent.can_maintain === false ||
              (label !== "Operation limits" &&
                agent.owner_kind === "org" &&
                agent.org_role !== "admin"))
          )
            continue;
          if (label === "Read my account" && agent.owner_kind === "org")
            continue;
          labels.push(label);
        }
        for (const [key, label] of Object.entries(
          AGENT_PROFILE_FIELDS[
            entry.section as keyof typeof AGENT_PROFILE_FIELDS
          ] ?? {},
        )) {
          if (
            (key === "name" && agent.kind === "nyxbot") ||
            (key === "description" &&
              entry.section !==
                (agent.kind === "nyxbot" ? "overview/persona" : "overview"))
          )
            continue;
          labels.push(label);
        }
      }
      expected.overview!.push(
        "Workspace setting",
        ...Object.values(ACCESS_MODE_LABEL),
      );
      if (agent.kind === "nyxbot")
        expected["overview/persona"]!.push(AGENT_NOTES_LABEL);
      labelsResolve(results, expected);
      const off = {
        voice: false,
        learning: false,
        orgChannels: false,
        owner: "",
      };
      const offAvailability = agentDetailsAvailability(agent, off);
      const disabled = entries(
        AGENT_DETAILS_SECTIONS.filter(
          (section) =>
            agentDetailsSections.available(section, offAvailability) ===
            section,
        ),
        AGENT_DETAILS_TITLES,
        "Scout",
        (section) => agentDetailsSearchTerms(section, agent, false, off),
      );
      for (const label of [
        "Device voice",
        "Automatic learning",
        "Threshold",
        "Learning proposals",
        "Model",
      ])
        expect(
          disabled.some((entry) => entry.terms.includes(translateUi(label))),
        ).toBe(false);
      if (agent.owner_kind === "org" && agent.org_role !== "admin")
        for (const label of [
          "Guest access",
          "Guest operation limits",
          "Read my account",
        ])
          expect(
            disabled.some((entry) => entry.terms.includes(translateUi(label))),
          ).toBe(false);
      if (agent.share_role)
        expect(disabled.map((entry) => entry.section)).toEqual([
          "overview",
          "grants",
          "grants/services",
          "skills",
          "memory",
        ]);
      if (agent.can_maintain === false)
        expect(disabled.map((entry) => entry.section)).not.toContain("memory");
      if (agent.owner_kind === "org" || agent.status === "destroyed")
        expect(disabled.map((entry) => entry.section)).not.toContain(
          "channels",
        );
    }
  });
}
