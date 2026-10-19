import { billingSearchSchema } from "@/schemas/billing";
import { parseTab, SETTINGS_TABS, SETTINGS_TAB_DEFAULT } from "@/lib/url-tabs";

export { ASSISTANT_SHELL_ROUTES, isAssistantShellRoute, type AssistantShellRoute } from "./shell-routes";
export type AccountPanel = "settings" | "billing" | "nyxbot";
export interface AccountPanelSearch {
  panel?: AccountPanel;
  panelTab?: string;
  panelPeriod?: ReturnType<typeof billingSearchSchema.parse>["period"];
  panelService?: string;
  panelServices?: string[];
  panelAction?: "topup";
}
export type AccountPanelParams = Omit<AccountPanelSearch, "panel">;
export const ACCOUNT_PANEL_KEYS = [
  "panel", "panelTab", "panelPeriod", "panelService", "panelServices", "panelAction",
] as const;

export const validateSettingsSearch = (search: Record<string, unknown>): { tab?: string } => ({
  ...(typeof search.tab === "string" ? { tab: search.tab } : {}),
});

export function billingPanelSearch(search: AccountPanelSearch) {
  return billingSearchSchema.parse({
    tab: search.panelTab,
    period: search.panelPeriod,
    service: search.panelService,
    services: search.panelServices,
    action: search.panelAction,
  });
}

export function parseAccountPanelSearch(search: Record<string, unknown>): AccountPanelSearch {
  if (search.panel === "settings") {
    const { tab } = validateSettingsSearch({ tab: search.panelTab });
    return { panel: "settings", ...(tab !== undefined ? {
      panelTab: parseTab(tab, SETTINGS_TABS, SETTINGS_TAB_DEFAULT),
    } : {}) };
  }
  if (search.panel === "nyxbot") return { panel: "nyxbot" };
  if (search.panel !== "billing") return {};
  const billing = billingSearchSchema.parse({
    tab: search.panelTab, period: search.panelPeriod,
    service: search.panelService, services: search.panelServices, action: search.panelAction,
  });
  return {
    panel: "billing",
    ...(billing.tab !== undefined ? { panelTab: billing.tab } : {}),
    ...(billing.period !== undefined ? { panelPeriod: billing.period } : {}),
    ...(billing.service !== undefined ? { panelService: billing.service } : {}),
    ...(billing.services !== undefined ? { panelServices: billing.services } : {}),
    ...(billing.action !== undefined ? { panelAction: billing.action } : {}),
  };
}

export function withoutAccountPanel(search: Record<string, unknown>) {
  const next = { ...search };
  for (const key of ACCOUNT_PANEL_KEYS) delete next[key];
  return next;
}

export function withAccountPanelSearch<T extends object>(parse: (search: Record<string, unknown>) => T) {
  return (search: Record<string, unknown>): T & AccountPanelSearch & { mock?: 1 } => ({
    ...parse(search),
    ...(search.mock === 1 || search.mock === "1" ? { mock: 1 as const } : {}),
    ...parseAccountPanelSearch(search),
  });
}
