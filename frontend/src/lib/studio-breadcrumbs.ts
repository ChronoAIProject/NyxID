import { translateUi } from "@/i18n/use-ui-translation";
import {
  SETTINGS_TABS,
  SETTINGS_TAB_DEFAULT,
  CONSENTS_TABS,
  CONSENTS_TAB_DEFAULT,
  INTEGRATION_GUIDE_TABS,
  INTEGRATION_GUIDE_TAB_DEFAULT,
  ORG_DETAIL_TABS,
  ORG_DETAIL_TAB_DEFAULT,
  KEYS_TABS,
  KEYS_TAB_DEFAULT,
  KEY_DETAIL_TABS,
  KEY_DETAIL_TAB_DEFAULT,
  INVITE_CODES_TABS,
  INVITE_CODES_TAB_DEFAULT,
  ADMIN_CREDITS_TABS,
  ADMIN_CREDITS_TAB_DEFAULT,
  ADMIN_BILLING_TABS,
  ADMIN_BILLING_TAB_DEFAULT,
  AI_SETUP_SKILL_TABS,
  parseTab,
} from "./url-tabs";
import { AI_TOOLS } from "./ai-tool-configs";

export interface StudioBreadcrumb {
  readonly label: string;
  readonly to?: string;
  readonly search?: Record<string, string>;
}

interface TabSection {
  readonly key: string;
  readonly values: readonly string[];
  readonly defaultValue: string;
  readonly labels: Readonly<Record<string, string>>;
}

function tabSection<T extends string>(
  values: readonly T[],
  defaultValue: T,
  labels: Readonly<Record<T, string>>,
  key = "tab",
): TabSection {
  return { key, values, defaultValue, labels };
}

const KEY_DETAIL_SECTION = tabSection(KEY_DETAIL_TABS, KEY_DETAIL_TAB_DEFAULT, {
  overview: "Overview",
  advanced: "Advanced",
  history: "History",
});
const ORG_SECTION = tabSection(ORG_DETAIL_TABS, ORG_DETAIL_TAB_DEFAULT, {
  members: "Members",
  "role-permissions": "Role permissions",
  invites: "Invites",
  approvals: "Approvals",
  "service-accounts": "Service Accounts",
  "developer-apps": "Developer Apps",
  settings: "Settings",
});
const TAB_SECTIONS: Readonly<Record<string, TabSection>> = {
  "/settings": tabSection(SETTINGS_TABS, SETTINGS_TAB_DEFAULT, {
    profile: "Profile",
    security: "Security",
    sessions: "Sessions",
    mcp: "MCP",
    display: "Display",
    privacy: "Privacy",
  }),
  "/settings/consents": tabSection(CONSENTS_TABS, CONSENTS_TAB_DEFAULT, {
    apps: "Authorized Apps",
    authorizations: "Authorizations",
  }),
  "/keys": tabSection(KEYS_TABS, KEYS_TAB_DEFAULT, {
    services: "External Services",
    pools: "Service Pools",
    nyxid: "Agent Keys",
  }),
  "/integration-guide": tabSection(
    INTEGRATION_GUIDE_TABS,
    INTEGRATION_GUIDE_TAB_DEFAULT,
    {
      react: "React SDK",
      core: "Core SDK",
      raw: "Raw API",
    },
  ),
  "/billing": tabSection(["billing", "usage"], "billing", {
    billing: "Billing",
    usage: "Usage",
  }),
  "/admin/usage": tabSection(["dashboard", "list"], "dashboard", {
    dashboard: "Dashboard",
    list: "List",
  }),
  "/admin/invite-codes": tabSection(
    INVITE_CODES_TABS,
    INVITE_CODES_TAB_DEFAULT,
    { codes: "Codes", users: "By User" },
    "view",
  ),
  "/admin/credits": tabSection(ADMIN_CREDITS_TABS, ADMIN_CREDITS_TAB_DEFAULT, {
    grants: "Credit grants",
    allowances: "Free allowances",
    schedules: "Schedules",
  }),
  "/admin/billing": tabSection(ADMIN_BILLING_TABS, ADMIN_BILLING_TAB_DEFAULT, {
    overview: "Services",
    rates: "Rate cache",
    settings: "Settings",
  }),
};

const PAGE_LABELS: Record<string, string> = {
  get "/dashboard"() { return translateUi("Dashboard"); },
  get "/billing"() { return translateUi("Billing & Usage"); },
  get "/keys"() { return translateUi("Services & Credentials"); },
  get "/tools"() { return translateUi("Tools"); },
  get "/admin/tools"() { return translateUi("Tools"); },
  get "/orgs"() { return translateUi("Organizations"); },
  get "/nodes"() { return translateUi("Credential Nodes"); },
  get "/channel-bots"() { return translateUi("Channel Bots"); },
  get "/onboarding/setups"() { return translateUi("Bot Setups"); },
  get "/channel-bots/connect"() { return translateUi("Setup links"); },
  get "/settings"() { return translateUi("Account Settings"); },
  get "/settings/consents"() { return translateUi("Access & Authorizations"); },
  get "/devices/onboard"() { return translateUi("Device Onboard"); },
  get "/approvals/settings"() { return translateUi("Notification Settings"); },
  get "/approvals/history"() { return translateUi("Approval History"); },
  get "/approvals/grants"() { return translateUi("Active Grants"); },
  get "/developer/apps"() { return translateUi("Developer Apps"); },
  get "/triggers"() { return translateUi("Triggers"); },
  get "/ai-setup"() { return translateUi("AI Setup Guide"); },
  get "/integration-guide"() { return translateUi("Integration & SDK Guide"); },
  get "/services"() { return translateUi("Services"); },
  get "/providers"() { return translateUi("Providers"); },
  get "/providers/manage"() { return translateUi("Providers"); },
  get "/providers/callback"() { return translateUi("Connection status"); },
  get "/admin"() { return translateUi("Admin"); },
  get "/admin/users"() { return translateUi("Users"); },
  get "/admin/audit-log"() { return translateUi("Audit Log"); },
  get "/admin/usage"() { return translateUi("Usage"); },
  get "/admin/integrity"() { return translateUi("Integrity"); },
  get "/admin/credits"() { return translateUi("Credits"); },
  get "/admin/billing"() { return translateUi("Billing"); },
  get "/admin/service-accounts"() { return translateUi("Service Accounts"); },
  get "/admin/oauth-clients"() { return translateUi("OAuth Clients"); },
  get "/admin/roles"() { return translateUi("Roles"); },
  get "/admin/groups"() { return translateUi("Groups"); },
  get "/admin/invite-codes"() { return translateUi("Invite Codes"); },
  get "/admin/feature-flags"() { return translateUi("Feature Flags"); },
  get "/admin/platform-credentials"() { return translateUi("Platform Credentials"); },
  get "/admin/platform-bots"() { return translateUi("Platform BotFather"); },
  get "/admin/upload-retention"() { return translateUi("Upload retention"); },
  get "/admin/nodes"() { return translateUi("Nodes"); },
  get "/admin/ownership"() { return translateUi("Ownership transfers"); },
};

const DETAIL_LABELS: Record<string, string> = {
  get "/keys"() { return translateUi("Credential"); },
  get "/nodes"() { return translateUi("Node"); },
  get "/services"() { return translateUi("Service"); },
  get "/providers"() { return translateUi("Provider"); },
  get "/developer/apps"() { return translateUi("Developer app"); },
  get "/admin/users"() { return translateUi("User"); },
  get "/admin/roles"() { return translateUi("Role"); },
  get "/admin/groups"() { return translateUi("Group"); },
  get "/admin/service-accounts"() { return translateUi("Service account"); },
};

export function buildStudioBreadcrumbs(
  pathname: string,
  labels: Readonly<Record<string, string>> = {},
  search: Readonly<Record<string, unknown>> = {},
  activeSection?: string,
  t: (key: string, options?: { readonly name: string }) => string = (key, options) =>
    options ? key.replaceAll("{{name}}", options.name) : key,
): readonly StudioBreadcrumb[] {
  const path = pathname.replace(/\/+$/, "") || "/";
  const named = (to: string, fallback: string): StudioBreadcrumb => ({
    label: labels[to] || t(fallback),
    to,
  });
  const page = (
    to: string,
    search?: Record<string, string>,
  ): StudioBreadcrumb => ({
    label: t(PAGE_LABELS[to]!),
    to,
    ...(search && { search }),
  });
  const finish = (items: StudioBreadcrumb[]): StudioBreadcrumb[] =>
    items.map((item, index) =>
      index === items.length - 1 ? { label: item.label } : item,
    );
  const subsection = (section: TabSection) => {
    const value = parseTab(
      activeSection ?? search[section.key],
      section.values,
      section.defaultValue,
    );
    return { label: t(section.labels[value]!) };
  };
  const keysParent = (tab: "services" | "nyxid") => [
    page("/keys", { tab: KEYS_TAB_DEFAULT }),
    {
      label: t(TAB_SECTIONS["/keys"]!.labels[tab]!),
      to: "/keys",
      search: { tab },
    },
  ];

  if (path === "/settings/devices/bind") {
    return finish([
      page("/settings", { tab: SETTINGS_TAB_DEFAULT }),
      { label: t("Security"), to: "/settings", search: { tab: "security" } },
      { label: t("Bind device") },
    ]);
  }
  if (path === "/channel-bots/connect") {
    return finish([page("/channel-bots"), { label: t("Setup links") }]);
  }
  const platformSetup = path.match(/^\/onboarding\/setups\/[^/]+$/);
  if (platformSetup) {
    return finish([page("/onboarding/setups"), named(path, "Bot setup")]);
  }
  if (path === "/providers/callback") {
    return finish([
      page("/keys", { tab: "services" }),
      { label: t("Connection status") },
    ]);
  }
  if (path === "/ai-setup") {
    const skills = AI_TOOLS.find(
      (tool) =>
        tool.id === search.skill && AI_SETUP_SKILL_TABS.includes(tool.id),
    );
    const mcp = AI_TOOLS.find((tool) => tool.id === search.tool);
    const selected = [
      skills && t("{{name}} skills", { name: skills.name }),
      mcp && t("{{name}} MCP", { name: mcp.name }),
    ].filter(Boolean);
    return selected.length
      ? finish([page(path), { label: selected.join(" · ") }])
      : [{ label: t(PAGE_LABELS[path]!) }];
  }
  const tabSection = TAB_SECTIONS[path];
  if (tabSection) {
    return finish([
      page(path, { [tabSection.key]: tabSection.defaultValue }),
      subsection(tabSection),
    ]);
  }
  if (PAGE_LABELS[path]) return [{ label: t(PAGE_LABELS[path]!) }];

  const apiKey = path.match(/^\/keys\/api-key\/[^/]+$/);
  if (apiKey) {
    return finish([...keysParent("nyxid"), named(path, "Agent key")]);
  }

  const conversation = path.match(
    /^(\/channel-bots\/[^/]+)\/conversations\/[^/]+$/,
  );
  if (conversation) {
    return finish([
      page("/channel-bots"),
      named(conversation[1]!, "Channel bot"),
      named(path, "Messages"),
    ]);
  }
  if (/^\/channel-bots\/[^/]+$/.test(path)) {
    return finish([page("/channel-bots"), named(path, "Channel bot")]);
  }

  if (/^\/orgs\/join\/[^/]+$/.test(path)) {
    return finish([page("/orgs"), { label: t("Join organization") }]);
  }
  const orgChild = path.match(
    /^(\/orgs\/[^/]+)\/(service-accounts|developer-apps)\/[^/]+$/,
  );
  if (orgChild) {
    const orgPath = orgChild[1]!;
    const isAccount = orgChild[2] === "service-accounts";
    return finish([
      page("/orgs"),
      {
        ...named(orgPath, "Organization"),
        search: { tab: ORG_DETAIL_TAB_DEFAULT },
      },
      {
        label: t(isAccount ? "Service Accounts" : "Developer Apps"),
        to: orgPath,
        search: { tab: orgChild[2]! },
      },
      named(path, isAccount ? "Service account" : "Developer app"),
    ]);
  }
  if (/^\/orgs\/[^/]+$/.test(path)) {
    return finish([
      page("/orgs"),
      {
        ...named(path, "Organization"),
        search: { tab: ORG_DETAIL_TAB_DEFAULT },
      },
      subsection(ORG_SECTION),
    ]);
  }
  if (/^\/keys\/services\/[^/]+$/.test(path)) {
    return finish([...keysParent("services"), named(path, "Service")]);
  }
  if (/^\/keys\/[^/]+$/.test(path)) {
    return finish([
      ...keysParent("services"),
      { ...named(path, "Credential"), search: { tab: KEY_DETAIL_TAB_DEFAULT } },
      subsection(KEY_DETAIL_SECTION),
    ]);
  }

  for (const [parent, fallback] of Object.entries(DETAIL_LABELS)) {
    if (!path.startsWith(`${parent}/`)) continue;
    const parts = path.slice(parent.length + 1).split("/");
    if (parts.length !== 1 && !(parts.length === 2 && parts[1] === "edit"))
      continue;
    const detailPath = `${parent}/${parts[0]}`;
    const parentPath = parent === "/providers" ? "/providers/manage" : parent;
    const items = [page(parentPath), named(detailPath, fallback)];
    if (parts[1] === "edit") {
      items.push({ label: t("Edit") });
      items[1] = { ...items[1]!, label: labels[path] || items[1]!.label };
    }
    return finish(items);
  }
  return [];
}
