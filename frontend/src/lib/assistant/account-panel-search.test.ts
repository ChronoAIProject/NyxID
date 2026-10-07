import { describe, expect, it } from "vitest";
import { billingSearchSchema } from "@/schemas/billing";
import { billingPanelSearch, parseAccountPanelSearch, withAccountPanelSearch, withoutAccountPanel } from "./account-panel-search";
import { parseAssistantSearch } from "./search";

describe("account panel search", () => {
  it.each([undefined, null, "nyxbot", "invalid", ["billing"], 1])("ignores invalid panel %s", (panel) => {
    expect(parseAccountPanelSearch({ panel, panelTab: "security" })).toEqual({});
  });
  it("uses shared settings validation and defaults invalid tabs", () => {
    expect(parseAccountPanelSearch({ panel: "settings", panelTab: "security", tab: "logins" })).toEqual({ panel: "settings", panelTab: "security" });
    expect(parseAccountPanelSearch({ panel: "settings", panelTab: "invalid" })).toEqual({ panel: "settings", panelTab: "profile" });
    expect(parseAccountPanelSearch({ panel: "settings", panelTab: ["security"] })).toEqual({ panel: "settings" });
  });
  it("delegates every billing field to its schema exactly", () => {
    const billing = { tab: "usage", period: "7d", service: " legacy ", services: [" a ", "a", "b", null, ""], action: "topup" };
    const panel = parseAccountPanelSearch({ panel: "billing", panelTab: billing.tab, panelPeriod: billing.period, panelService: billing.service, panelServices: billing.services, panelAction: billing.action });
    expect(billingPanelSearch(panel)).toEqual(billingSearchSchema.parse(billing));
    expect(parseAccountPanelSearch({ panel: "billing", panelTab: "security", panelPeriod: "bad", panelAction: "bad", panelServices: "a" })).toEqual({ panel: "billing" });
  });
  it("preserves route-owned keys and mock through composition and removal", () => {
    const search = { tab: "logins", machine: "n", mock: "1", panel: "billing", panelTab: "usage", panelPeriod: "7d", panelServices: ["a"], panelAction: "topup" };
    const parsed = withAccountPanelSearch((raw) => ({ tab: raw.tab, machine: raw.machine }))(search);
    expect(parsed).toMatchObject({ tab: "logins", machine: "n", mock: 1, panel: "billing", panelTab: "usage" });
    expect(withoutAccountPanel({ ...parsed })).toEqual({ tab: "logins", machine: "n", mock: 1 });
    expect(parseAssistantSearch({ c: "chat", draft: true, agent: "agent", g: "group", mock: 1, panel: "settings" })).toEqual({ c: "chat", draft: true, agent: "agent", g: "group", mock: 1, panel: "settings" });
  });
});
