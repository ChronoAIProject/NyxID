import { translateApp } from "../../i18n/appTranslation";
import type { AssistantConversation } from "../../features/assistant/assistantTypes";
import type { AssistantAgent } from "../../features/agents/agentContracts";
import type { Group } from "../../features/groups/groupContracts";
import type {
  WorkspaceContentParamList,
  WorkspaceSheetParamList,
} from "../../app/navigationTypes";
import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import {
  workspaceRecents,
  type WorkspaceRecent,
} from "./workspaceSidebarModel";
import { threadTitle } from "./workspaceChatListModel";

type RouteTarget<List> = {
  [Name in keyof List]: { name: Name; params?: List[Name] };
}[keyof List];
export type WorkspaceSearchTarget =
  | { kind: "chat"; target: WorkspaceChatTarget }
  | ({ kind: "page" } & RouteTarget<WorkspaceContentParamList>)
  | ({ kind: "settings" } & RouteTarget<WorkspaceSheetParamList>)
  | { kind: "theme"; preference: "light" | "dark" | "system" };
export type WorkspaceSearchEntry = {
  key: string;
  label: string;
  hint?: string;
  keywords?: readonly string[];
  target: WorkspaceSearchTarget;
};
export type WorkspaceSearchSection = {
  title: string;
  data: WorkspaceSearchEntry[];
};

/** Same all-words match as web assistant-search; opaque route/row IDs never match. */
export function workspaceSearchMatches(
  query: string,
  ...text: (string | null | undefined)[]
) {
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const haystack = text.filter(Boolean).join(" ").toLocaleLowerCase();
  return words.every((word) => haystack.includes(word));
}

const pages: WorkspaceSearchEntry[] = [
  {
    key: "page:automations",
    get label() {
      return translateApp("Automations");
    },
    keywords: ["schedule", "webhook", "trigger"],
    target: { kind: "page", name: "Automations" },
  },
  {
    key: "page:machines",
    get label() {
      return translateApp("Machines");
    },
    keywords: ["node", "computer"],
    target: { kind: "page", name: "Machines" },
  },
  {
    key: "page:logins",
    get label() {
      return translateApp("Saved logins");
    },
    keywords: ["password", "machine"],
    target: { kind: "page", name: "Machines", params: { tab: "logins" } },
  },
  {
    key: "page:plugins",
    get label() {
      return translateApp("Plugins");
    },
    keywords: ["skills", "connections", "connectors"],
    target: { kind: "page", name: "Plugins" },
  },
  {
    key: "page:activity",
    get label() {
      return translateApp("Activity");
    },
    keywords: ["approvals", "requests", "history"],
    target: { kind: "page", name: "Activity" },
  },
  {
    key: "page:agents",
    get label() {
      return translateApp("Agents & Crews");
    },
    keywords: ["manage", "groups"],
    target: { kind: "page", name: "Agents" },
  },
  {
    key: "page:create-agent",
    get label() {
      return translateApp("Create new agent");
    },
    target: { kind: "page", name: "AgentCreate" },
  },
  {
    key: "page:recents",
    get label() {
      return translateApp("All chats");
    },
    keywords: ["recent", "history", "threads"],
    target: { kind: "page", name: "WorkspaceRecents" },
  },
  {
    key: "quick:scanner",
    get label() {
      return translateApp("Scan QR code");
    },
    get keywords() {
      return ["camera", translateApp("quick action")] as const;
    },
    target: {
      kind: "settings",
      name: "DeviceLogin",
      params: { start_scanner: true },
    },
  },
  {
    key: "quick:terminal",
    get label() {
      return translateApp("Terminal login code");
    },
    get keywords() {
      return ["device", translateApp("quick action")] as const;
    },
    target: { kind: "settings", name: "AgentKeyLogin", params: { mint: true } },
  },
];
const settings: WorkspaceSearchEntry[] = [
  {
    key: "settings:profile",
    get label() {
      return translateApp("Settings: Profile");
    },
    keywords: ["name", "email", "account"],
    target: { kind: "settings", name: "AccountSettings" },
  },
  {
    key: "settings:security",
    get label() {
      return translateApp("Settings: Account & security");
    },
    get keywords() {
      return [
      "password",
      "mfa",
      "two-factor",
      "passkey",
      "sessions",
      "devices",
        translateApp("sign out"),
    ] as const;
    },
    target: { kind: "settings", name: "AccountSecurity" },
  },
  ...(["display", "privacy", "notifications"] as const).map((section) => ({
    key: `settings:${section}`,
    label: translateApp(section === "notifications" ? "Settings: Notifications" : section === "display" ? "Settings: Display" : section === "privacy" ? "Settings: Privacy" : section === "voice" ? "Settings: Voice" : "Settings: Profile"),
    keywords:
      section === "display"
        ? ["theme", "appearance", translateApp("text size"), "motion"]
        : section === "privacy"
          ? ["data", "telemetry", "delete"]
          : ["push", "delivery"],
    target: {
      kind: "settings" as const,
      name: "AccountSettings" as const,
      params: { section },
    },
  })),
  {
    key: "settings:usage",
    get label() {
      return translateApp("Usage & credits");
    },
    keywords: ["billing", "wallet", "cost", "tokens"],
    target: { kind: "settings", name: "UsageCredits" },
  },
  {
    key: "settings:terms",
    get label() {
      return translateApp("Terms of Service");
    },
    keywords: ["legal"],
    target: { kind: "settings", name: "TermsOfService" },
  },
  {
    key: "settings:privacy-policy",
    get label() {
      return translateApp("Privacy Policy");
    },
    keywords: ["legal"],
    target: { kind: "settings", name: "PrivacyPolicy" },
  },
];

export function workspaceSearchSections(
  query: string,
  conversations: readonly AssistantConversation[],
  agents: readonly AssistantAgent[],
  crews: readonly Group[],
): WorkspaceSearchSection[] {
  const typing = !!query.trim();
  const availableAgents = agents.filter(
    (agent) => agent.status !== "destroyed" && agent.can_use !== false,
  );
  const names = new Map(
    availableAgents.map((agent) => [
      agent.id,
      agent.display_name || agent.name,
    ]),
  );
  const seen = new Set<string>();
  const denied = new Set(agents.filter((agent) => agent.can_use === false).map((agent) => agent.id));
  const chats = conversations.filter((chat) => !chat.agent || !denied.has(chat.agent.id))
    .sort((a, b) => b.last_message_at.localeCompare(a.last_message_at))
    .filter((chat) => {
      if (seen.has(chat.id)) return false;
      seen.add(chat.id);
      return true;
    });
  const recent: WorkspaceSearchEntry[] = typing
    ? chats.map((chat) => ({
        key: `chat:${chat.id}`,
        label: threadTitle(chat),
        hint:
          (chat.agent && names.get(chat.agent.id)) ||
          chat.agent?.display_name ||
          chat.agent?.name ||
          "NyxBot",
        keywords: [chat.agent?.name ?? ""],
        target: {
          kind: "chat",
          target: {
            kind: "agent",
            agentId: chat.agent?.id,
            conversationId: chat.id,
          },
        },
      }))
    : workspaceRecents(chats, crews)
        .slice(0, 8)
        .map((row) => ({
          key: row.key,
          label: row.title,
          hint:
            row.target.kind === "group"
              ? translateApp("Crew")
              : row.target.kind === "agent"
                ? names.get(row.target.agentId ?? "")
                : undefined,
          target: { kind: "chat", target: row.target },
        }));
  const sections: WorkspaceSearchSection[] = [
    { title: typing ? translateApp("Chats") : translateApp("Recent chats"), data: recent },
    ...(typing
      ? [
          {
            title: translateApp("Agents"),
            data: availableAgents.map(
              (agent): WorkspaceSearchEntry => ({
                key: `agent:${agent.id}`,
                label: agent.display_name || agent.name,
                hint: agent.kind === "nyxbot" ? "NyxBot" : `@${agent.name}`,
                keywords: [agent.name, "agent", agent.owner_name ?? ""],
                target: {
                  kind: "chat",
                  target: { kind: "agent", agentId: agent.id },
                },
              }),
            ),
          },
          {
            title: translateApp("Crews"),
            data: crews.map(
              (crew): WorkspaceSearchEntry => ({
                key: `crew:${crew.id}`,
                label: crew.name,
                keywords: ["group", "crew"],
                target: {
                  kind: "chat",
                  target: { kind: "group", groupId: crew.id },
                },
              }),
            ),
          },
        ]
      : []),
    { title: translateApp("Go to"), data: pages },
    ...(typing
      ? [
          { title: translateApp("Settings"), data: settings },
          {
            title: translateApp("Appearance"),
            data: (["light", "dark", "system"] as const).map(
              (preference): WorkspaceSearchEntry => ({
                key: `theme:${preference}`,
                label: translateApp(preference === "light" ? "Theme: Light" : preference === "dark" ? "Theme: Dark" : "Theme: System"),
                keywords: [
                  "appearance",
                  "mode",
                  preference === "system" ? "auto" : "",
                ],
                target: { kind: "theme", preference },
              }),
            ),
          },
        ]
      : []),
  ];
  return sections
    .map((section) => ({
      ...section,
      data: section.data.filter((entry) =>
        workspaceSearchMatches(
          query,
          entry.label,
          entry.hint,
          ...(entry.keywords ?? []),
        ),
      ),
    }))
    .filter((section) => section.data.length);
}

export function filterWorkspaceRecents(
  rows: readonly WorkspaceRecent[],
  query: string,
  agentId?: string,
) {
  return rows.filter(
    (row) =>
      (!agentId ||
        (row.target.kind === "agent" && row.target.agentId === agentId)) &&
      workspaceSearchMatches(query, row.title),
  );
}
