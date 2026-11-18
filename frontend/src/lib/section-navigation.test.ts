import { sectionNavigation } from "./section-navigation";
import { describe, expect, it } from "vitest";
import { ACCOUNT_SETTINGS_SECTIONS, accountSettingsSections } from "./account-settings-sections";
import { BILLING_SECTIONS } from "./billing-sections";
import {
  AGENT_DETAILS_SECTIONS,
  agentDetailsSections,
  agentDetailsAvailability,
} from "./assistant/agent-details-sections";
import { parseAccountPanelSearch, validateSettingsSearch } from "./assistant/account-panel-search";
import { parseAssistantSearch } from "./assistant/search";
import { parseNyxBotSettingsSection } from "./assistant/nyxbot-settings-sections";
import { billingSearchSchema } from "@/schemas/billing";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";

for (const [name, sections] of [
  ["account", ACCOUNT_SETTINGS_SECTIONS],
  ["billing", BILLING_SECTIONS],
  ["agent", AGENT_DETAILS_SECTIONS],
] as const) {
  const paths = sectionNavigation(sections as readonly string[]);
  describe(`${name} section paths`, () => {
    it("derives every parent and deep-link ancestor from its path", () => {
      for (const section of sections) {
        const parts = section.split("/");
        expect(paths.ancestry(section)).toEqual(
          parts.map((_, index) => parts.slice(0, index + 1).join("/")),
        );
        expect(paths.parent(section)).toBe(
          parts.length > 1 ? parts.slice(0, -1).join("/") : undefined,
        );
        expect(paths.normalize(`${section}/unknown`)).toBe(section);
      }
    });
    it("drops malformed paths and falls back past any unavailable ancestor", () => {
      for (const malformed of [
        null,
        [],
        1,
        "",
        "/display",
        "display/",
        "display//text",
        "Display",
        "display%2Ftext",
      ])
        expect(paths.normalize(malformed)).toBeUndefined();
      const nested =
        sections.find((section) => section.split("/").length === 3) ??
        sections.find((section) => section.includes("/"))!;
      const parent = paths.parent(nested)!;
      expect(paths.available(nested, { [parent]: false })).toBe(paths.parent(parent));
      expect(paths.available(nested, { [nested]: false })).toBe(parent);
    });
  });
}

it("round-trips the old ids and new slash paths without consuming unrelated URL state", () => {
  for (const tab of ACCOUNT_SETTINGS_SECTIONS) {
    expect(parseAccountPanelSearch({ panel: "settings", panelTab: tab })).toEqual({
      panel: "settings",
      panelTab: tab,
    });
    expect(validateSettingsSearch({ tab })).toEqual({ tab });
  }
  for (const details of AGENT_DETAILS_SECTIONS)
    expect(parseAssistantSearch({ agent: "a", details })).toEqual({ agent: "a", details });
  for (const section of BILLING_SECTIONS) {
    const tab = section.split("/")[0];
    const filters = { tab, section, period: "7d", services: ["service"], action: "topup" };
    expect(billingSearchSchema.parse(filters)).toMatchObject(filters);
    expect(
      parseAccountPanelSearch({
        panel: "billing",
        panelTab: tab,
        panelSection: section,
        panelPeriod: "7d",
        panelServices: ["service"],
        panelAction: "topup",
      }),
    ).toMatchObject({
      panel: "billing",
      panelTab: tab,
      panelSection: section,
      panelPeriod: "7d",
      panelServices: ["service"],
      panelAction: "topup",
    });
  }
});

it("matches feature, ownership and read-only availability for agent search and navigation", () => {
  const agent = {
    kind: "specialist",
    owner_kind: "person",
    status: "idle",
    services: [],
    can_maintain: true,
  } as unknown as AssistantAgent;
  const options = { orgChannels: false, voice: false, learning: false, owner: "owner" };
  expect(agentDetailsSections.available("grants/machines", agentDetailsAvailability({ ...agent, share_role: "user" }, options))).toBe("grants");
  expect(agentDetailsAvailability({ ...agent, share_role: "user" }, options)["overview/voice"]).toBe(false);
  expect(
    agentDetailsAvailability(
      { ...agent, owner_kind: "org", status: "destroyed" },
      { ...options, voice: true, orgChannels: true },
    ).channels,
  ).toBe(false);
});

it.each([
  [
    "nyxbot",
    "permissions/specialists",
    "permissions"
  ],
  [
    "nyxbot",
    "tasks/specialists",
    "tasks"
  ],
  [
    "nyxbot",
    "tasks/groups",
    "tasks"
  ],
  [
    "nyxbot",
    "automations/budgets",
    "automations"
  ],
  [
    "agent",
    "overview/access",
    "overview"
  ],
  [
    "agent",
    "overview/chat",
    "overview"
  ],
  [
    "agent",
    "overview/ownership",
    "overview"
  ],
  [
    "agent",
    "grants/operations",
    "grants"
  ],
  [
    "agent",
    "grants/guests",
    "grants"
  ],
  [
    "agent",
    "grants/voice",
    "grants"
  ],
  [
    "agent",
    "skills/installed",
    "skills"
  ],
  [
    "agent",
    "skills/learning",
    "skills"
  ],
  [
    "settings",
    "security/mfa",
    "security"
  ],
  [
    "settings",
    "security/password",
    "security"
  ],
  [
    "settings",
    "security/linked-accounts",
    "security"
  ],
  [
    "settings",
    "mcp/cursor",
    "mcp"
  ],
  [
    "settings",
    "mcp/claude-code",
    "mcp"
  ],
  [
    "settings",
    "mcp/codex",
    "mcp"
  ],
  [
    "settings",
    "display/appearance",
    "display"
  ],
  [
    "settings",
    "display/text",
    "display"
  ],
  [
    "settings",
    "display/chat",
    "display"
  ],
  [
    "settings",
    "display/sidebar",
    "display"
  ],
  [
    "settings",
    "display/colors/surfaces",
    "display/colors"
  ],
  [
    "settings",
    "display/colors/text",
    "display/colors"
  ],
  [
    "settings",
    "display/colors/controls",
    "display/colors"
  ],
  [
    "settings",
    "display/readability",
    "display"
  ],
  [
    "billing",
    "billing/benefits/wallet",
    "billing/benefits"
  ],
  [
    "billing",
    "billing/benefits/grants",
    "billing/benefits"
  ],
  [
    "billing",
    "billing/benefits/allowances",
    "billing/benefits"
  ],
  [
    "billing",
    "usage/summary",
    "usage"
  ],
  [
    "billing",
    "usage/charts/quantities",
    "usage/charts"
  ],
  [
    "billing",
    "usage/charts/funding",
    "usage/charts"
  ]
])("normalizes removed %s section %s to %s", (surface, removed, expected) => {
  if (surface === "nyxbot") {
    expect(parseNyxBotSettingsSection(removed)).toBe(expected);
    expect(parseAccountPanelSearch({ panel: "nyxbot", panelTab: removed }).panelTab).toBe(expected);
  } else if (surface === "settings") {
    expect(accountSettingsSections.normalize(validateSettingsSearch({ tab: removed }).tab)).toBe(expected);
    expect(parseAccountPanelSearch({ panel: "settings", panelTab: removed }).panelTab).toBe(expected);
  } else if (surface === "agent") {
    expect(parseAssistantSearch({ agent: "a", details: removed }).details).toBe(expected);
  } else {
    const tab = removed.split("/")[0];
    expect(billingSearchSchema.parse({ tab, section: removed }).section).toBe(expected);
    expect(parseAccountPanelSearch({ panel: "billing", panelTab: tab, panelSection: removed }).panelSection).toBe(expected);
  }
});
