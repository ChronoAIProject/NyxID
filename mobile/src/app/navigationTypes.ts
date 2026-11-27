import type {
  CompositeNavigationProp,
  CompositeScreenProps,
  NavigatorScreenParams,
} from "@react-navigation/native";
import type {
  NativeStackNavigationProp,
  NativeStackScreenProps,
} from "@react-navigation/native-stack";
import type { AgentProfileField } from "../features/agents/AgentProfileEditScreen";
import type { UsageCreditsRouteParams } from "../features/billing/UsageCreditsScreen";
import type { MachineDesktopRouteParams } from "../features/machines/desktopEntry";
import type { ConnectionHistoryGroup } from "../features/plugins/pluginConnectionHistoryTypes";

export type AssistantParams = {
  home?: boolean;
  /** Local identity of an unsent compose session; never sent to the server. */
  draftId?: string;
  conversationId?: string;
  /** Resolve any persisted segment ID to its displayed transcript row. */
  messageId?: string;
  acknowledgementId?: string;
  agentId?: string;
  draft?: boolean;
  openLatest?: boolean;
};

export type WorkspaceParamList = {
  Assistant: AssistantParams | undefined;
  Automations: { agentId?: string } | undefined;
  Machines: { machine?: string; tab?: "logins" } | undefined;
  Services: undefined;
  Plugins: { tab?: "connectors" | "skills"; agentId?: string } | undefined;
  Agents: { tab?: "agents" | "groups" } | undefined;
  Activity: { challengeId?: string } | undefined;
  /** Stacked approval inbox used while a terminal remains pending. */
  ActivityReview: undefined;
  /** The owner's durable notification feed (bell); hidden while the flag is off. */
  Notifications: undefined;
  AccountSettings:
    | { section?: "display" | "privacy" | "notifications" }
    | undefined;
};

export type SkillPageParams = {
  agentId: string;
  skillId: string;
  name: string;
  description?: string | null;
  version?: string;
};

export type AuthoredSkillPageParams = {
  agentId: string;
  conversationId: string;
  acknowledgementId: string;
};

type SharedEntryParamList = {
  TermsOfService: undefined;
  PrivacyPolicy: undefined;
  OpenSourceNotices: undefined;
  DeviceLogin: { user_code?: string; start_scanner?: boolean; staging_scan?: true } | undefined;
  AgentKeyLogin:
    | { user_code?: string; flow?: "agent-key" | "device"; mint?: boolean }
    | undefined;
};

/** Every workspace destination pushes horizontally inside the content slab. */
export type WorkspaceContentParamList = Omit<
  WorkspaceParamList,
  "AccountSettings"
> & {
  AssistantContext: AssistantParams | undefined;
  GroupChat: { groupId: string };
  AssistantDetails: { agentId?: string; conversationId?: string; draftKey?: string; draftEpoch?: number };
  AssistantTitleEdit: { conversationId: string; agentId?: string };
  AssistantHistory: { agentId?: string } | undefined;
  AssistantWorkingContext: { conversationId: string };
  WorkspaceSearch: undefined;
  WorkspaceRecents: { agentId?: string } | undefined;
  AgentCreate: undefined;
  AgentDetail: { agentId: string };
  AgentProfileEdit: { agentId: string; field: AgentProfileField };
  AgentAvatarEdit: { agentId: string };
  GroupEditor: { groupId?: string } | undefined;
  SshTerminal: {
    /** Catalog service id used by the terminal WebSocket and deep links. */
    serviceId: string;
    /** Exact UserService connection id when opened from Services/Plugins. */
    connectionId?: string;
    principal?: string;
    /** Only Services/Plugins may set this boolean; link parsing never does. */
    autoConnect?: boolean;
  };
  ServiceSetup: { connectionId?: string; catalogSlug?: string };
  AgentSkills: { agentId: string };
  SkillDetail: SkillPageParams;
  SkillVersions: SkillPageParams;
  SkillHistory: SkillPageParams;
  AuthoredSkillReview: AuthoredSkillPageParams;
  AgentMachines: { agentId: string };
  AgentMachineEditor: { agentId: string; machineId: string };
  MachineDetail: { machineId: string };
  PluginConnections: { connectorId: string };
  /** Metadata-only change set from a connection's History tab. */
  ConnectionHistoryDetail: { title: string; group: ConnectionHistoryGroup };
  ActiveGrants: undefined;
  MachinePolicy: { machineId: string; ownerId: string };
  MachineActivity: { machineId: string; ownerId: string; agent?: string };
  MachineActivityFilter: {
    machineId: string;
    ownerId: string;
    agent: string;
    agents: { id: string; label: string }[];
  };
  MachineActivityDetail: {
    id: string;
    action: string;
    outcome: string;
    agent: string;
    createdAt: string;
    durationMs?: number;
    exitCode?: number;
  };
  MachineSavedLogin: { loginId: string; ownerLabel: string };
  AutomationDetail: { id: string };
  AutomationRuns: { id: string };
  AutomationRunDetail: { id: string; runId: string };
  AutomationAgentFilter: { agentId?: string } | undefined;
  ActivityDetail: { challengeId: string; entryPoint?: string };
};

/** Only Settings and its account/quick-action pages use the pull-up surface. */
export type WorkspaceSheetParamList = SharedEntryParamList & {
  AccountSettings: WorkspaceParamList["AccountSettings"];
  AccountSecurity: undefined;
  UsageCredits: UsageCreditsRouteParams | undefined;
  NotificationSettings: undefined;
  DisplaySettings: undefined;
  LanguageSettings: undefined;
  /** The Duo rail's quick-access list (saved to the account). */
  QuickAccessSettings: undefined;
  /** `voice` false: only dictation settings (no live voice for this account). */
  VoiceSettings: { voice?: boolean } | undefined;
  AppleWatchSettings: undefined;
  PrivacySettings: undefined;
  ProfileEdit: { field: "display_name" | "avatar_url" };
  SessionDetail: { sessionId: string };
  ChatLayoutEdit: { key: "columnWidth" | "composerMaxHeight" };
  /** The folded Duo quick rail's Recent chats. */
  RecentChats: undefined;
};

export type RootStackParamList = SharedEntryParamList & {
  Auth: undefined;
  Workspace: NavigatorScreenParams<WorkspaceContentParamList> | undefined;
  MachineDesktop: MachineDesktopRouteParams;
  WorkspaceSheet: NavigatorScreenParams<WorkspaceSheetParamList> | undefined;
};

export type SheetScreenProps<Name extends keyof WorkspaceSheetParamList> =
  CompositeScreenProps<
    NativeStackScreenProps<WorkspaceSheetParamList, Name>,
    NativeStackScreenProps<RootStackParamList>
  >;
export type ContentScreenProps<Name extends keyof WorkspaceContentParamList> =
  CompositeScreenProps<
    NativeStackScreenProps<WorkspaceContentParamList, Name>,
    NativeStackScreenProps<RootStackParamList>
  >;
export type SheetNavigationProp = CompositeNavigationProp<
  NativeStackNavigationProp<WorkspaceSheetParamList>,
  NativeStackNavigationProp<RootStackParamList>
>;
export type WorkspaceScreenProps<Name extends keyof WorkspaceParamList> =
  Name extends "AccountSettings"
    ? SheetScreenProps<"AccountSettings">
    : ContentScreenProps<Extract<Name, keyof WorkspaceContentParamList>>;
export type WorkspaceNavigationProp = CompositeNavigationProp<
  NativeStackNavigationProp<WorkspaceContentParamList>,
  SheetNavigationProp
>;

export const workspaceContentRouteNames: (keyof WorkspaceContentParamList)[] = [
  "Assistant",
  "AssistantContext",
  "GroupChat",
  "AssistantDetails",
  "AssistantTitleEdit",
  "AssistantHistory",
  "AssistantWorkingContext",
  "Automations",
  "Agents",
  "Plugins",
  "Services",
  "SshTerminal",
  "Machines",
  "Activity",
  "ActivityReview",
  "Notifications",
  "WorkspaceSearch",
  "WorkspaceRecents",
  "AgentCreate",
  "AgentDetail",
  "AgentProfileEdit",
  "AgentAvatarEdit",
  "GroupEditor",
  "ServiceSetup",
  "AgentSkills",
  "SkillDetail",
  "SkillVersions",
  "SkillHistory",
  "AuthoredSkillReview",
  "AgentMachines",
  "AgentMachineEditor",
  "MachineDetail",
  "PluginConnections",
  "ConnectionHistoryDetail",
  "ActiveGrants",
  "MachinePolicy",
  "MachineActivity",
  "MachineActivityFilter",
  "MachineActivityDetail",
  "MachineSavedLogin",
  "AutomationDetail",
  "AutomationRuns",
  "AutomationRunDetail",
  "AutomationAgentFilter",
  "ActivityDetail",
];
export const workspaceSettingsRouteNames: (keyof WorkspaceSheetParamList)[] = [
  "AccountSettings",
  "AccountSecurity",
  "UsageCredits",
  "NotificationSettings",
  "DisplaySettings",
  "LanguageSettings",
  "QuickAccessSettings",
  "VoiceSettings",
  "AppleWatchSettings",
  "PrivacySettings",
  "DeviceLogin",
  "AgentKeyLogin",
  "RecentChats",
  "TermsOfService",
  "PrivacyPolicy",
  "OpenSourceNotices",
];
