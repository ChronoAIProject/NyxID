import type { LinkingOptions } from "@react-navigation/native";
import type { RootStackParamList } from "./navigationTypes";

const assistantParams = {
  conversationId: String,
  acknowledgementId: String,
  agentId: String,
  draft: (value: string) => value === "true",
  openLatest: (value: string) => value === "true",
};

// Shared signed-out entry URLs remain root routes; authenticated entry wrappers
// present them in WorkspaceSheet. Avoid duplicate link patterns in both stacks.
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
        AssistantContext: {
          path: "chat/:conversationId",
          parse: assistantParams,
        },
        GroupChat: { path: "groups/:groupId", parse: { groupId: String } },
        WorkspaceSearch: "search",
        WorkspaceRecents: "recents",
        Automations: {
          path: "automations",
          alias: ["assistant/automations"],
          parse: { agentId: String },
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
        Machines: {
          path: "assistant/machines",
          parse: {
            machine: String,
            tab: (value: string) => (value === "logins" ? "logins" : undefined),
          },
        },
        Services: "services",
        Activity: { path: "activity", parse: { challengeId: String } },
        AgentDetail: { path: "agents/:agentId", parse: { agentId: String } },
        GroupEditor: {
          path: "groups/edit/:groupId?",
          parse: { groupId: String },
        },
        AutomationDetail: { path: "automations/:id", parse: { id: String } },
        AutomationRuns: { path: "automations/:id/runs", parse: { id: String } },
      },
    },
    WorkspaceSheet: {
      initialRouteName: "AccountSettings",
      screens: {
        AccountSettings: "account",
        AccountSecurity: "account/security",
        UsageCredits: {
          path: "account/usage",
          parse: {
            period: (value: string) =>
              ["24h", "7d", "30d"].includes(value) ? value : "30d",
            ownerId: String,
          },
        },
      },
    },

    MachineDesktop: {
      path: "assistant/machines/:nodeId/desktop",
      parse: {
        nodeId: String,
        conversation_id: String,
        context_id: String,
        display: String,
      },
    },

    DeviceLogin: { path: "login/device", parse: { user_code: String } },
    AgentKeyLogin: { path: "login/agent-key", parse: { user_code: String } },
    TermsOfService: "terms",
    PrivacyPolicy: "privacy",
  },
};
