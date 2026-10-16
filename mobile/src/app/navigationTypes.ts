import type {
  CompositeNavigationProp,
  CompositeScreenProps,
  NavigatorScreenParams,
} from "@react-navigation/native";
import type {
  BottomTabNavigationProp,
  BottomTabScreenProps,
} from "@react-navigation/bottom-tabs";
import type {
  NativeStackNavigationProp,
  NativeStackScreenProps,
} from "@react-navigation/native-stack";

export type AssistantParams = {
  conversationId?: string;
  acknowledgementId?: string;
  agentId?: string;
  draft?: boolean;
  openLatest?: boolean;
};

export type WorkspaceParamList = {
  Assistant: AssistantParams | undefined;
  Automations: { agentId?: string; setup?: string } | undefined;
  Services: undefined;
  Plugins: { tab?: "connectors" | "skills"; agentId?: string } | undefined;
  Agents: { tab?: "agents" | "groups" } | undefined;
  Activity: { challengeId?: string } | undefined;
  AccountSettings: undefined;
};

export type RootStackParamList = {
  Auth: undefined;
  Workspace: NavigatorScreenParams<WorkspaceParamList> | undefined;
  AssistantContext: AssistantParams | undefined;
  AgentDetail: { agentId: string };
  GroupChat: { groupId: string };
  GroupEditor: { groupId?: string } | undefined;
  ServiceSetup: { connectionId?: string; catalogSlug?: string };
  AgentSkills: { agentId: string };
  AutomationDetail: { id: string };
  AutomationEditor:
    | { id?: string; agentId?: string; setup?: string }
    | undefined;
  AutomationRuns: { id: string };
  AutomationPreferences: undefined;
  ActivityDetail: { challengeId: string };
  AccountSecurity: undefined;
  TermsOfService: undefined;
  PrivacyPolicy: undefined;
  DeviceLogin: { user_code?: string; start_scanner?: boolean } | undefined;
  AgentKeyLogin:
    | { user_code?: string; flow?: "agent-key" | "device"; mint?: boolean }
    | undefined;
};

export type WorkspaceScreenProps<Name extends keyof WorkspaceParamList> =
  CompositeScreenProps<
    BottomTabScreenProps<WorkspaceParamList, Name>,
    NativeStackScreenProps<RootStackParamList>
  >;

export type WorkspaceNavigationProp = CompositeNavigationProp<
  BottomTabNavigationProp<WorkspaceParamList>,
  NativeStackNavigationProp<RootStackParamList>
>;
