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
    label: "Automations",
    keywords: ["schedule", "webhook", "trigger"],
    target: { kind: "page", name: "Automations" },
  },
  {
    key: "page:machines",
    label: "Machines",
    keywords: ["node", "computer"],
    target: { kind: "page", name: "Machines" },
  },
  {
    key: "page:logins",
    label: "Saved logins",
    keywords: ["password", "machine"],
    target: { kind: "page", name: "Machines", params: { tab: "logins" } },
  },
  {
    key: "page:plugins",
    label: "Plugins",
    keywords: ["skills", "connections", "connectors"],
    target: { kind: "page", name: "Plugins" },
  },
  {
    key: "page:activity",
    label: "Activity",
    keywords: ["approvals", "requests", "history"],
    target: { kind: "page", name: "Activity" },
  },
  {
    key: "page:agents",
    label: "Agents & Crews",
    keywords: ["manage", "groups"],
    target: { kind: "page", name: "Agents" },
  },
  {
    key: "page:create-agent",
    label: "Create new agent",
    target: { kind: "page", name: "AgentCreate" },
  },
  {
    key: "page:recents",
    label: "All chats",
    keywords: ["recent", "history", "threads"],
    target: { kind: "page", name: "WorkspaceRecents" },
  },
  {
    key: "quick:scanner",
    label: "Scan QR code",
    keywords: ["camera", "quick action"],
    target: {
      kind: "settings",
      name: "DeviceLogin",
      params: { start_scanner: true },
    },
  },
  {
    key: "quick:terminal",
    label: "Terminal login code",
    keywords: ["device", "quick action"],
    target: { kind: "settings", name: "AgentKeyLogin", params: { mint: true } },
  },
];
const settings: WorkspaceSearchEntry[] = [
  {
    key: "settings:profile",
    label: "Settings: Profile",
    keywords: ["name", "email", "account"],
    target: { kind: "settings", name: "AccountSettings" },
  },
  {
    key: "settings:security",
    label: "Settings: Account & security",
    keywords: [
      "password",
      "mfa",
      "two-factor",
      "passkey",
      "sessions",
      "devices",
      "sign out",
    ],
    target: { kind: "settings", name: "AccountSecurity" },
  },
  ...(["display", "privacy", "notifications"] as const).map((section) => ({
    key: `settings:${section}`,
    label: `Settings: ${section[0]!.toUpperCase()}${section.slice(1)}`,
    keywords:
      section === "display"
        ? ["theme", "appearance", "text size", "motion"]
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
    label: "Usage & credits",
    keywords: ["billing", "wallet", "cost", "tokens"],
    target: { kind: "settings", name: "UsageCredits" },
  },
  {
    key: "settings:terms",
    label: "Terms of Service",
    keywords: ["legal"],
    target: { kind: "settings", name: "TermsOfService" },
  },
  {
    key: "settings:privacy-policy",
    label: "Privacy Policy",
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
    (agent) => agent.status !== "destroyed",
  );
  const names = new Map(
    availableAgents.map((agent) => [
      agent.id,
      agent.display_name || agent.name,
    ]),
  );
  const seen = new Set<string>();
  const chats = [...conversations]
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
    : workspaceRecents(conversations, crews)
        .slice(0, 8)
        .map((row) => ({
          key: row.key,
          label: row.title,
          hint:
            row.target.kind === "group"
              ? "Crew"
              : row.target.kind === "agent"
                ? names.get(row.target.agentId ?? "")
                : undefined,
          target: { kind: "chat", target: row.target },
        }));
  const sections: WorkspaceSearchSection[] = [
    { title: typing ? "Chats" : "Recent chats", data: recent },
    ...(typing
      ? [
          {
            title: "Agents",
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
            title: "Crews",
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
    { title: "Go to", data: pages },
    ...(typing
      ? [
          { title: "Settings", data: settings },
          {
            title: "Appearance",
            data: (["light", "dark", "system"] as const).map(
              (preference): WorkspaceSearchEntry => ({
                key: `theme:${preference}`,
                label: `Theme: ${preference[0]!.toUpperCase()}${preference.slice(1)}`,
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
