import type { LinkingOptions } from "@react-navigation/native";
import type { RootStackParamList } from "./navigationTypes";

const assistantParams = {
  conversationId: String,
  acknowledgementId: String,
  agentId: String,
  draft: (value: string) => value === "true",
  openLatest: (value: string) => value === "true",
};

// Keep this configuration free of native modules so links can be checked with
// React Navigation's parser without initializing notifications or the app.
export const navigationLinkingConfig: NonNullable<
  LinkingOptions<RootStackParamList>["config"]
> = {
  initialRouteName: "Workspace",
  screens: {
    Auth: "auth",
    Workspace: {
      initialRouteName: "Assistant",
      screens: {
        Assistant: { path: "chat", parse: assistantParams },
        Automations: {
          path: "automations",
          alias: ["assistant/automations"],
          parse: { agentId: String, setup: String },
        },
        Agents: {
          path: "agents",
          parse: {
            tab: (value: string) => (value === "groups" ? "groups" : "agents"),
          },
        },
        Plugins: {
          path: "plugins",
          parse: { tab: String, agentId: String },
        },
        Services: "services",
        Activity: { path: "activity", parse: { challengeId: String } },
        AccountSettings: "account",
      },
    },
    AssistantContext: {
      path: "chat/:conversationId",
      parse: assistantParams,
    },
    AgentDetail: { path: "agents/:agentId", parse: { agentId: String } },
    GroupChat: { path: "groups/:groupId", parse: { groupId: String } },
    GroupEditor: { path: "groups/edit/:groupId?", parse: { groupId: String } },
    AutomationDetail: { path: "automations/:id", parse: { id: String } },
    AutomationEditor: {
      path: "automations/edit/:id?",
      parse: { id: String, agentId: String, setup: String },
    },
    AutomationRuns: { path: "automations/:id/runs", parse: { id: String } },
    AutomationPreferences: "automations/preferences",
    AccountSecurity: "account/security",
    DeviceLogin: { path: "login/device", parse: { user_code: String } },
    AgentKeyLogin: { path: "login/agent-key", parse: { user_code: String } },
    TermsOfService: "terms",
    PrivacyPolicy: "privacy",
  },
};
