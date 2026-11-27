// Each loader points to a module shipped in the app. React Navigation calls
// it synchronously when rendering a destination, so startup can skip its setup.
export const screenModules = {
  workspaceBrowse: (): typeof import("../components/workspace/WorkspaceBrowseScreen") =>
    require("../components/workspace/WorkspaceBrowseScreen"),
  workspaceRecentChatsSheet: (): typeof import("../components/workspace/WorkspaceRecentChatsSheet") =>
    require("../components/workspace/WorkspaceRecentChatsSheet"),
  accountSecurity: (): typeof import("../features/account/AccountSecurityScreen") =>
    require("../features/account/AccountSecurityScreen"),
  accountSettings: (): typeof import("../features/account/AccountSettingsScreen") =>
    require("../features/account/AccountSettingsScreen"),
  appleWatchSettings: (): typeof import("../features/account/AppleWatchSettingsScreen") =>
    require("../features/account/AppleWatchSettingsScreen"),
  displaySettings: (): typeof import("../features/account/DisplaySettingsScreen") =>
    require("../features/account/DisplaySettingsScreen"),
  languageSettings: (): typeof import("../features/account/LanguageSettingsScreen") =>
    require("../features/account/LanguageSettingsScreen"),
  notificationSettings: (): typeof import("../features/account/NotificationSettingsScreen") =>
    require("../features/account/NotificationSettingsScreen"),
  openSourceNotices: (): typeof import("../features/account/OpenSourceNoticesScreen") =>
    require("../features/account/OpenSourceNoticesScreen"),
  privacySettings: (): typeof import("../features/account/PrivacySettingsScreen") =>
    require("../features/account/PrivacySettingsScreen"),
  sessionDetail: (): typeof import("../features/account/SessionDetailScreen") =>
    require("../features/account/SessionDetailScreen"),
  voiceSettings: (): typeof import("../features/account/VoiceSettingsScreen") =>
    require("../features/account/VoiceSettingsScreen"),
  activityDetail: (): typeof import("../features/activity/ActivityDetailScreen") =>
    require("../features/activity/ActivityDetailScreen"),
  activity: (): typeof import("../features/activity/ActivityScreen") =>
    require("../features/activity/ActivityScreen"),
  assistantDetails: (): typeof import("../features/assistant/AssistantDetailsScreen") =>
    require("../features/assistant/AssistantDetailsScreen"),
  automations: (): typeof import("../features/automations/AutomationsScreen") =>
    require("../features/automations/AutomationsScreen"),
  usageCredits: (): typeof import("../features/billing/UsageCreditsScreen") =>
    require("../features/billing/UsageCreditsScreen"),
  agentMachines: (): typeof import("../features/machines/AgentMachinesScreen") =>
    require("../features/machines/AgentMachinesScreen"),
  machineActivity: (): typeof import("../features/machines/MachineActivityScreen") =>
    require("../features/machines/MachineActivityScreen"),
  machineDesktopRoute: (): typeof import("../features/machines/MachineDesktopRoute") =>
    require("../features/machines/MachineDesktopRoute"),
  machinePolicy: (): typeof import("../features/machines/MachinePolicyScreen") =>
    require("../features/machines/MachinePolicyScreen"),
  machines: (): typeof import("../features/machines/MachinesScreen") =>
    require("../features/machines/MachinesScreen"),
  savedLogins: (): typeof import("../features/machines/SavedLogins") =>
    require("../features/machines/SavedLogins"),
  notifications: (): typeof import("../features/notifications/NotificationsScreen") =>
    require("../features/notifications/NotificationsScreen"),
  nativeAuthoredSkillReview: (): typeof import("../features/plugins/NativeAuthoredSkillReview") =>
    require("../features/plugins/NativeAuthoredSkillReview"),
  pluginConnectionHistory: (): typeof import("../features/plugins/PluginConnectionHistory") =>
    require("../features/plugins/PluginConnectionHistory"),
  pluginSkills: (): typeof import("../features/plugins/PluginSkills") =>
    require("../features/plugins/PluginSkills"),
  quickAccessSettings: (): typeof import("../features/quickAccess/QuickAccessSettingsScreen") =>
    require("../features/quickAccess/QuickAccessSettingsScreen"),
  sshTerminal: (): typeof import("../features/ssh/SshTerminalScreen") =>
    require("../features/ssh/SshTerminalScreen"),
};
