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
};

const PAGE_LABELS: Record<string, string> = {
  "/dashboard": "Dashboard",
  "/billing": "Billing & Usage",
  "/keys": "Services & Credentials",
  "/tools": "Tools",
  "/admin/tools": "Tools",
  "/orgs": "Organizations",
  "/nodes": "Credential Nodes",
  "/channel-bots": "Channel Bots",
  "/channel-bots/connect": "Setup links",
  "/settings": "Account Settings",
  "/settings/consents": "Access & Authorizations",
  "/devices/onboard": "Device Onboard",
  "/approvals/settings": "Notification Settings",
  "/approvals/history": "Approval History",
  "/approvals/grants": "Active Grants",
  "/developer/apps": "Developer Apps",
  "/triggers": "Triggers",
  "/ai-setup": "AI Setup Guide",
  "/integration-guide": "Integration & SDK Guide",
  "/services": "Services",
  "/providers": "Providers",
  "/providers/manage": "Providers",
  "/providers/callback": "Connection status",
  "/admin": "Admin",
  "/admin/users": "Users",
  "/admin/audit-log": "Audit Log",
  "/admin/usage": "Usage",
  "/admin/integrity": "Integrity",
  "/admin/credits": "Credits",
  "/admin/service-accounts": "Service Accounts",
  "/admin/oauth-clients": "OAuth Clients",
  "/admin/roles": "Roles",
  "/admin/groups": "Groups",
  "/admin/invite-codes": "Invite Codes",
  "/admin/feature-flags": "Feature Flags",
  "/admin/platform-credentials": "Platform Credentials",
  "/admin/upload-retention": "Upload retention",
  "/admin/nodes": "Nodes",
  "/admin/ownership": "Ownership transfers",
};

const DETAIL_LABELS: Record<string, string> = {
  "/keys": "Credential",
  "/nodes": "Node",
  "/services": "Service",
  "/providers": "Provider",
  "/developer/apps": "Developer app",
  "/admin/users": "User",
  "/admin/roles": "Role",
  "/admin/groups": "Group",
  "/admin/service-accounts": "Service account",
};

export function buildStudioBreadcrumbs(
  pathname: string,
  labels: Readonly<Record<string, string>> = {},
  search: Readonly<Record<string, unknown>> = {},
  activeSection?: string,
): readonly StudioBreadcrumb[] {
  const path = pathname.replace(/\/+$/, "") || "/";
  const named = (to: string, fallback: string): StudioBreadcrumb => ({
    label: labels[to] || fallback,
    to,
  });
  const page = (
    to: string,
    search?: Record<string, string>,
  ): StudioBreadcrumb => ({
    label: PAGE_LABELS[to]!,
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
    return { label: section.labels[value]! };
  };
  const keysParent = (tab: "services" | "nyxid") => [
    page("/keys", { tab: KEYS_TAB_DEFAULT }),
    {
      label: TAB_SECTIONS["/keys"]!.labels[tab]!,
      to: "/keys",
      search: { tab },
    },
  ];

  if (path === "/settings/devices/bind") {
    return finish([
      page("/settings", { tab: SETTINGS_TAB_DEFAULT }),
      { label: "Security", to: "/settings", search: { tab: "security" } },
      { label: "Bind device" },
    ]);
  }
  if (path === "/channel-bots/connect") {
    return finish([page("/channel-bots"), { label: "Setup links" }]);
  }
  if (path === "/providers/callback") {
    return finish([
      page("/keys", { tab: "services" }),
      { label: "Connection status" },
    ]);
  }
  if (path === "/ai-setup") {
    const skills = AI_TOOLS.find(
      (tool) =>
        tool.id === search.skill && AI_SETUP_SKILL_TABS.includes(tool.id),
    );
    const mcp = AI_TOOLS.find((tool) => tool.id === search.tool);
    const selected = [
      skills && `${skills.name} skills`,
      mcp && `${mcp.name} MCP`,
    ].filter(Boolean);
    return selected.length
      ? finish([page(path), { label: selected.join(" · ") }])
      : [{ label: PAGE_LABELS[path]! }];
  }
  const tabSection = TAB_SECTIONS[path];
  if (tabSection) {
    return finish([
      page(path, { [tabSection.key]: tabSection.defaultValue }),
      subsection(tabSection),
    ]);
  }
  if (PAGE_LABELS[path]) return [{ label: PAGE_LABELS[path] }];

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
    return finish([page("/orgs"), { label: "Join organization" }]);
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
        label: isAccount ? "Service Accounts" : "Developer Apps",
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
      items.push({ label: "Edit" });
      items[1] = { ...items[1]!, label: labels[path] || items[1]!.label };
    }
    return finish(items);
  }
  return [];
}
