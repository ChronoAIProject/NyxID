import { LanguageSettingsScreen } from "../features/account/LanguageSettingsScreen";
import { useAppLanguage } from "../i18n/LanguageProvider";
import { WorkspacePageSurface } from "./WorkspacePageSurface";
import { workspaceContentAnimation } from "../components/workspace/workspacePushState";
import {
  AssistantDetailsScreen,
  AssistantTitleEditScreen,
  AssistantWorkingContextScreen,
} from "../features/assistant/AssistantDetailsScreen";
import {
  AgentProfileEditScreen,
  agentProfileFieldTitle,
} from "../features/agents/AgentProfileEditScreen";
import {
  WorkspaceRecentsScreen,
  WorkspaceSearchScreen,
} from "../components/workspace/WorkspaceBrowseScreen";
import { UsageCreditsScreen } from "../features/billing/UsageCreditsScreen";
import { WorkspaceInteractionSurface } from "../lib/accessibility/WorkspaceInteractionSurface";
import { workspaceFocus } from "../lib/accessibility/workspaceFocus";
import {
  useCallback,
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  usePreventRemove,
  type NavigationAction,
} from "@react-navigation/native";
import {
  createNativeStackNavigator,
  type NativeStackScreenProps,
} from "@react-navigation/native-stack";
import { AuthHomeScreen } from "../features/auth/AuthHomeScreen";
import { DeviceLoginScreen } from "../features/auth/DeviceLoginScreen";
import { AgentKeyLoginScreen } from "../features/auth/AgentKeyLoginScreen";
import { useAuthSession } from "../features/auth/AuthSessionContext";
import { AccountSettingsScreen } from "../features/account/AccountSettingsScreen";
import { DisplaySettingsScreen } from "../features/account/DisplaySettingsScreen";
import { VoiceSettingsScreen } from "../features/account/VoiceSettingsScreen";
import { NotificationSettingsScreen } from "../features/account/NotificationSettingsScreen";
import { PrivacySettingsScreen } from "../features/account/PrivacySettingsScreen";
import {
  ProfileFieldEditScreen,
  profileFieldTitle,
} from "../features/account/ProfileFieldEditScreen";
import { SessionDetailScreen } from "../features/account/SessionDetailScreen";
import {
  ChatLayoutEditScreen,
  chatLayoutTitle,
} from "../features/display/ChatLayoutEditScreen";
import { AccountSecurityScreen } from "../features/account/AccountSecurityScreen";
import { MachinePolicyScreen } from "../features/machines/MachinePolicyScreen";
import {
  MachineActivityDetailScreen,
  MachineActivityFilterScreen,
  MachineActivityScreen,
} from "../features/machines/MachineActivityScreen";
import { SavedLoginScreen } from "../features/machines/SavedLogins";
import { AgentMachinesScreen } from "../features/machines/AgentMachinesScreen";
import { MachineDesktopRoute } from "../features/machines/MachineDesktopRoute";
import { MachinesScreen } from "../features/machines/MachinesScreen";
import { WorkspacePageHeader } from "../components/workspace/WorkspacePageHeader";
import { WorkspaceDrawerHeader } from "../components/workspace/WorkspaceDrawerHeader";
import { ActivityScreen } from "../features/activity/ActivityScreen";
import { PluginSkillPage } from "../features/plugins/PluginSkills";
import { AuthoredSkillPage } from "../features/plugins/NativeAuthoredSkillReview";
import { ConnectionHistoryDetailScreen } from "../features/plugins/PluginConnectionHistory";
import { ActivityDetailScreen } from "../features/activity/ActivityDetailScreen";
import { AssistantScreen } from "../features/assistant/AssistantScreen";
import { AssistantLinkHost } from "../features/assistant/AssistantLinkHost";
import { ServicesScreen } from "../features/services/ServicesScreen";
import { AgentsScreen } from "../features/agents/AgentsScreen";
import { AgentsGroupsScreen } from "../features/agents/AgentsGroupsScreen";
import { GroupChatScreen } from "../features/groups/GroupChatScreen";
import { GroupEditorScreen } from "../features/groups/GroupEditorScreen";
import {
  AutomationsScreen,
  AutomationDetailScreen,
  AutomationRunsScreen,
  AutomationRunDetailScreen,
  AutomationAgentFilterScreen,
} from "../features/automations/AutomationsScreen";
import {
  PluginsScreen,
  AgentSkillsScreen,
} from "../features/plugins/PluginsScreen";
import { TermsOfServiceScreen } from "../features/legal/TermsOfServiceScreen";
import { PrivacyPolicyScreen } from "../features/legal/PrivacyPolicyScreen";
import { FullScreenLoading } from "../components/FullScreenLoading";
import { useReducedMotion } from "../lib/accessibility/useReducedMotion";
import {
  WorkspaceSheetSurface,
  type WorkspaceSheetHandle,
} from "../components/workspace/WorkspaceSheetSurface";
import { useTheme } from "../theme/ThemeContext";
import { darkSheetColors, lightSheetColors } from "../theme/mobileTheme";
import { fonts } from "../theme/designTokens";
import {
  WorkspaceSessionContext,
  WorkspaceEdgeBackContext,
  type WorkspaceEdgeBackChange,
  WorkspaceMenuContext,
  WorkspaceNavigationCancelledContext,
  WorkspaceSheetContext,
  useOpenWorkspaceChat,
  WorkspaceNavigationGuardContext,
  type WorkspaceNavigationGuardChange,
  workspaceHeaderOptions,
} from "./WorkspaceHeader";
import {
  openWorkspacePageAction,
  dismissWorkspaceSheetAction,
  workspaceSharedEntryNavigationAction,
} from "./navigationHierarchy";
import {
  workspaceContextChatNavigationAction,
  workspaceChatNavigationAction,
} from "./workspaceChatNavigation";
import type {
  RootStackParamList,
  WorkspaceContentParamList,
  ContentScreenProps,
  WorkspaceSheetParamList,
  WorkspaceScreenProps,
  AssistantParams,
} from "./navigationTypes";

export type {
  RootStackParamList,
  WorkspaceParamList,
  WorkspaceScreenProps,
  WorkspaceNavigationProp,
} from "./navigationTypes";

const Stack = createNativeStackNavigator<RootStackParamList>();
const ContentStack = createNativeStackNavigator<WorkspaceContentParamList>();
const SheetStack = createNativeStackNavigator<WorkspaceSheetParamList>();

function AgentsRoute({ navigation, route }: WorkspaceScreenProps<"Agents">) {
  const openChat = useOpenWorkspaceChat();
  return (
    <AgentsGroupsScreen
      tab={route.params?.tab === "groups" ? "groups" : "agents"}
      onTabChange={(tab) => navigation.setParams({ tab })}
      onOpenGroup={(groupId) => navigation.push("GroupChat", { groupId })}
      onCreateAgent={() => navigation.push("AgentCreate")}
      onCreateGroup={() => navigation.push("GroupEditor")}
      onOpenChat={openChat}
      onOpenSkills={(agentId) => navigation.push("AgentSkills", { agentId })}
      onEditProfile={(agentId, field) =>
        navigation.push("AgentProfileEdit", { agentId, field })
      }
      onOpenMachines={(agentId) =>
        navigation.push("AgentMachines", { agentId })
      }
      onOpenAutomations={(agentId) =>
        navigation.navigate({
          name: "Automations",
          params: { agentId },
          merge: true,
        })
      }
    />
  );
}

function GroupChatRoute({
  navigation,
  route,
}: ContentScreenProps<"GroupChat">) {
  return (
    <WorkspaceInteractionSurface style={{ flex: 1 }}>
      <AssistantLinkHost scope={route.params.groupId}>
        <GroupChatScreen
          key={route.params.groupId}
          groupId={route.params.groupId}
          onBack={() => navigation.goBack()}
          onSettings={() =>
            navigation.dispatch(
              openWorkspacePageAction("GroupEditor", {
                groupId: route.params.groupId,
              }),
            )
          }
          onOpenConversation={(params) =>
            navigation.push("AssistantContext", params)
          }
          onOpenAgent={(agentId) =>
            navigation.dispatch(
              openWorkspacePageAction("AgentDetail", { agentId }),
            )
          }
        />
      </AssistantLinkHost>
    </WorkspaceInteractionSurface>
  );
}

function GroupEditorRoute({
  navigation,
  route,
}: ContentScreenProps<"GroupEditor">) {
  return (
    <GroupEditorScreen
      key={route.params?.groupId ?? "new"}
      groupId={route.params?.groupId}
      onBack={() => navigation.goBack()}
      onSaved={(groupId) => {
        if (route.params?.groupId) navigation.goBack();
        else navigation.push("GroupChat", { groupId });
      }}
      onRemoved={() =>
        navigation.popTo("Agents", { tab: "groups" }, { merge: true })
      }
    />
  );
}

function AgentDetailRoute({
  navigation,
  route,
}: ContentScreenProps<"AgentDetail">) {
  const openChat = useOpenWorkspaceChat();
  return (
    <AgentsScreen
      agentId={route.params.agentId}
      onClose={() => navigation.goBack()}
      onOpenChat={openChat}
      onOpenSkills={(agentId) => navigation.push("AgentSkills", { agentId })}
      onEditProfile={(agentId, field) =>
        navigation.push("AgentProfileEdit", { agentId, field })
      }
      onOpenMachines={(agentId) =>
        navigation.push("AgentMachines", { agentId })
      }
      onOpenAutomations={(agentId) =>
        navigation.popTo("Automations", { agentId }, { merge: true })
      }
    />
  );
}

function AgentCreateRoute({ navigation }: ContentScreenProps<"AgentCreate">) {
  return (
    <AgentsScreen
      initialCreate
      onClose={() => navigation.goBack()}
      onCreated={(agentId) => navigation.replace("AgentDetail", { agentId })}
    />
  );
}

function AssistantContextRoute({
  route,
}: ContentScreenProps<"AssistantContext">) {
  return (
    <WorkspaceInteractionSurface style={{ flex: 1 }}>
      <AssistantLinkHost
        scope={`${route.key}:${route.params?.conversationId ?? route.params?.draftId ?? "draft"}`}
      >
        <AssistantScreen contextual route={route} />
      </AssistantLinkHost>
    </WorkspaceInteractionSurface>
  );
}

function AssistantRoute({ route }: WorkspaceScreenProps<"Assistant">) {
  return (
    <AssistantLinkHost
      scope={`${route.key}:${route.params?.conversationId ?? route.params?.draftId ?? "home"}`}
    >
      <AssistantScreen route={route} />
    </AssistantLinkHost>
  );
}

function PluginsRoute({ route }: WorkspaceScreenProps<"Plugins">) {
  return (
    <PluginsScreen
      initialTab={route.params?.tab}
      agentId={route.params?.agentId}
    />
  );
}

function PluginConnectionsRoute({
  route,
}: ContentScreenProps<"PluginConnections">) {
  return <PluginsScreen connectorId={route.params.connectorId} />;
}

function ServiceSetupRoute({
  navigation,
  route,
}: ContentScreenProps<"ServiceSetup">) {
  return (
    <ServicesScreen
      key={route.key}
      initialConnectionId={route.params.connectionId}
      initialCatalogSlug={route.params.catalogSlug}
      onClose={() => navigation.goBack()}
    />
  );
}

function AgentSkillsRoute({ route }: ContentScreenProps<"AgentSkills">) {
  return (
    <AgentSkillsScreen
      key={route.params.agentId}
      agentId={route.params.agentId}
    />
  );
}

/** Home, chats and workspace pages share the horizontally pushed content slab. */
const WorkspaceRootSwapContext = createContext(false);
const WorkspacePagePaintedContext = createContext<(identity: string) => void>(
  () => {},
);

function WorkspaceNavigator() {
  const { t } = useAppLanguage();
  const rootSwap = useContext(WorkspaceRootSwapContext);
  const onPainted = useContext(WorkspacePagePaintedContext);
  const onOpenMenu = useContext(WorkspaceMenuContext);
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  return (
    <ContentStack.Navigator
      initialRouteName="Assistant"
      screenOptions={({ navigation }) => ({
        ...workspaceHeaderOptions({
          role: "back",
          onBack: navigation.goBack,
          onOpenMenu,
        }),
        header: WorkspacePageHeader,
        headerStyle: { backgroundColor: colors.bg },
        headerTintColor: colors.textPrimary,
        contentStyle: { backgroundColor: colors.bg },
        headerShadowVisible: false,
        animation: workspaceContentAnimation(reducedMotion, rootSwap),
        animationTypeForReplace: "push",
      })}
      screenLayout={({ children, route }) => (
        <WorkspacePageSurface
          identity={JSON.stringify([route.key, route.name, route.params])}
          onPainted={onPainted}
        >
          {children}
        </WorkspacePageSurface>
      )}
    >
      <ContentStack.Screen
        name="Assistant"
        component={AssistantRoute}
        initialParams={{ home: true }}
      />
      <ContentStack.Screen
        name="AssistantContext"
        component={AssistantContextRoute}
      />
      <ContentStack.Screen name="GroupChat" component={GroupChatRoute} />
      <ContentStack.Screen
        name="AssistantDetails"
        component={AssistantDetailsScreen}
        options={{ title: t("Chat details") }}
      />
      <ContentStack.Screen
        name="AssistantTitleEdit"
        component={AssistantTitleEditScreen}
        options={{ title: t("Title") }}
      />
      <ContentStack.Screen
        name="AssistantHistory"
        component={WorkspaceRecentsScreen}
        options={{ title: t("Chat history") }}
      />
      <ContentStack.Screen
        name="AssistantWorkingContext"
        component={AssistantWorkingContextScreen}
        options={{ title: t("Working context") }}
      />
      <ContentStack.Group
        screenOptions={{
          ...workspaceHeaderOptions({ role: "menu", onOpenMenu }),
          gestureEnabled: false,
        }}
      >
        <ContentStack.Screen
          name="Automations"
          component={AutomationsScreen}
          options={{ title: t("Automations") }}
        />
        <ContentStack.Screen
          name="Agents"
          component={AgentsRoute}
          options={{ title: t("Agents & Crews") }}
        />
        <ContentStack.Screen
          name="Plugins"
          component={PluginsRoute}
          options={{ title: t("Plugins") }}
        />
        <ContentStack.Screen
          name="Services"
          component={ServicesScreen}
          options={{ title: t("Services") }}
        />
        <ContentStack.Screen
          name="Machines"
          component={MachinesScreen}
          options={{ title: t("Machines") }}
        />
        <ContentStack.Screen
          name="Activity"
          component={ActivityScreen}
          options={{ title: t("Activity") }}
        />
        <ContentStack.Screen
          name="WorkspaceSearch"
          component={WorkspaceSearchScreen}
          options={{ title: t("Search") }}
        />
        <ContentStack.Screen
          name="WorkspaceRecents"
          component={WorkspaceRecentsScreen}
          options={{ title: t("Recents") }}
        />
      </ContentStack.Group>
      <ContentStack.Screen
        name="AgentCreate"
        component={AgentCreateRoute}
        options={{ title: t("New agent") }}
      />
      <ContentStack.Screen
        name="AgentDetail"
        component={AgentDetailRoute}
        options={{ title: t("Agent") }}
      />
      <ContentStack.Screen
        name="AgentProfileEdit"
        component={AgentProfileEditScreen}
        options={({ route }) => ({
          title: agentProfileFieldTitle(route.params.field),
        })}
      />
      <ContentStack.Screen
        name="GroupEditor"
        component={GroupEditorRoute}
        options={({ route }) => ({
          title: route.params?.groupId ? t("Crew settings") : t("New crew"),
        })}
      />
      <ContentStack.Screen
        name="ServiceSetup"
        component={ServiceSetupRoute}
        options={({ route }) => ({
          title: route.params.catalogSlug
            ? t("Connect service")
            : t("Manage connection"),
        })}
      />
      <ContentStack.Screen
        name="MachineDetail"
        component={MachinesScreen}
        options={{ title: t("Machine") }}
      />
      <ContentStack.Screen
        name="AgentMachineEditor"
        component={AgentMachinesScreen}
        options={{ title: t("Machine access") }}
      />
      <ContentStack.Screen
        name="PluginConnections"
        component={PluginConnectionsRoute}
        options={{ title: t("Connections") }}
      />
      <ContentStack.Screen
        name="ConnectionHistoryDetail"
        component={ConnectionHistoryDetailScreen}
        options={({ route }) => ({ title: route.params.title })}
      />
      <ContentStack.Screen
        name="ActiveGrants"
        component={ActivityScreen}
        options={{ title: t("Active grants") }}
      />
      <ContentStack.Screen
        name="MachinePolicy"
        component={MachinePolicyScreen}
        options={{
          title: t("Owner policy and updates"),
          header: WorkspacePageHeader,
        }}
      />
      <ContentStack.Screen
        name="MachineActivity"
        component={MachineActivityScreen}
        options={{
          title: t("Machine activity"),
          header: WorkspacePageHeader,
        }}
      />
      <ContentStack.Screen
        name="MachineActivityFilter"
        component={MachineActivityFilterScreen}
        options={{ title: t("Agent"), header: WorkspacePageHeader }}
      />
      <ContentStack.Screen
        name="MachineActivityDetail"
        component={MachineActivityDetailScreen}
        options={{ title: t("Activity"), header: WorkspacePageHeader }}
      />
      <ContentStack.Screen
        name="MachineSavedLogin"
        component={SavedLoginScreen}
        options={{ title: t("Saved login"), header: WorkspacePageHeader }}
      />
      <ContentStack.Screen
        name="AgentMachines"
        component={AgentMachinesScreen}
        options={{
          title: t("Agent machine access"),
          header: WorkspacePageHeader,
        }}
      />
      <ContentStack.Screen
        name="AgentSkills"
        component={AgentSkillsRoute}
        options={{ title: t("Agent skills") }}
      />
      <ContentStack.Screen
        name="SkillDetail"
        component={PluginSkillPage}
        options={({ route }) => ({ title: route.params.name })}
      />
      <ContentStack.Screen
        name="SkillVersions"
        component={PluginSkillPage}
        options={{ title: t("Skill versions") }}
      />
      <ContentStack.Screen
        name="SkillHistory"
        component={PluginSkillPage}
        options={{ title: t("Skill history") }}
      />
      <ContentStack.Screen
        name="AuthoredSkillReview"
        component={AuthoredSkillPage}
        options={{ title: t("Review skill draft") }}
      />
      <ContentStack.Screen
        name="AutomationDetail"
        component={AutomationDetailScreen}
        options={{ title: t("Automation") }}
      />
      <ContentStack.Screen
        name="AutomationRuns"
        component={AutomationRunsScreen}
        options={{ title: t("Run history") }}
      />
      <ContentStack.Screen
        name="AutomationRunDetail"
        component={AutomationRunDetailScreen}
        options={{ title: t("Run") }}
      />
      <ContentStack.Screen
        name="AutomationAgentFilter"
        component={AutomationAgentFilterScreen}
        options={{ title: t("Agent") }}
      />
      <ContentStack.Screen
        name="ActivityDetail"
        component={ActivityDetailScreen}
        options={{ title: t("Approval detail") }}
      />
    </ContentStack.Navigator>
  );
}

function SettingsSheetRoute({
  navigation,
  route,
}: NativeStackScreenProps<RootStackParamList, "WorkspaceSheet">) {
  const { t } = useAppLanguage();
  // The route renders outside SheetThemeProvider, so header colors read the drawer palette directly.
  const { mode } = useTheme();
  const colors = mode === "light" ? lightSheetColors : darkSheetColors;
  const onOpenMenu = useContext(WorkspaceMenuContext);
  const controller = useRef<WorkspaceSheetHandle | null>(null);
  const guards = useRef(new Map<symbol, () => Promise<boolean>>());
  const [dismissAuthorized, setDismissAuthorized] = useState(false);
  const pendingExit = useRef<NavigationAction | undefined>(undefined);
  const reducedMotion = useReducedMotion();
  useEffect(() => () => workspaceFocus.releaseDrawer(route.key), [route.key]);
  const registerDismissGuard = useCallback(
    (id: symbol, guard: () => Promise<boolean>) => {
      guards.current.set(id, guard);
      return () => {
        guards.current.delete(id);
      };
    },
    [],
  );
  const canClose = useCallback(async () => {
    for (const guard of [...guards.current.values()])
      if (!(await guard())) return false;
    setDismissAuthorized(true);
    return true;
  }, []);
  const onClose = useCallback(() => {
    pendingExit.current = undefined;
    controller.current?.dismiss();
  }, []);
  const sessionActive = useContext(WorkspaceSessionContext);
  usePreventRemove(sessionActive && !dismissAuthorized, ({ data }) => {
    pendingExit.current = data.action;
    controller.current?.dismiss();
  });
  const [finishRequested, setFinishRequested] = useState(false);
  const finishClose = useCallback(() => setFinishRequested(true), []);
  useEffect(() => {
    if (!dismissAuthorized || !finishRequested) return;
    const action =
      pendingExit.current ?? dismissWorkspaceSheetAction(navigation.getState());
    pendingExit.current = undefined;
    if (action) navigation.dispatch(action);
  }, [navigation, dismissAuthorized, finishRequested]);
  const onOpenChat = useCallback(
    (params?: AssistantParams) => {
      const action = workspaceContextChatNavigationAction(
        navigation.getState(),
        params ?? {},
      );
      if (action) navigation.dispatch(action);
    },
    [navigation],
  );
  const onOpenGroup = useCallback(
    (groupId: string) => {
      const action = workspaceChatNavigationAction(navigation.getState(), {
        kind: "group",
        groupId,
      });
      if (action) navigation.dispatch(action);
    },
    [navigation],
  );
  const canHandleHardwareBack = useCallback(() => {
    const sheet = navigation
      .getState()
      .routes.find((item) => item.key === route.key);
    return (sheet?.state?.routes.length ?? 1) <= 1;
  }, [navigation, route.key]);
  const context = useMemo(
    () => ({
      registerDismissGuard,
      dismissAuthorized,
      rootKey: navigation.getState().key,
      sheetKey: route.key,
      onClose,
      onOpenChat,
      onOpenGroup,
    }),
    [
      navigation,
      route.key,
      onClose,
      onOpenChat,
      onOpenGroup,
      registerDismissGuard,
      dismissAuthorized,
    ],
  );
  return (
    <WorkspaceSheetContext.Provider value={context}>
      <WorkspaceSheetSurface
        onClose={finishClose}
        canClose={canClose}
        controller={controller}
        canHandleHardwareBack={canHandleHardwareBack}
      >
        <SheetStack.Navigator
          initialRouteName="AccountSettings"
          screenOptions={({ navigation, route }) => ({
            ...workspaceHeaderOptions({
              role: "back",
              onBack: navigation.goBack,
              onOpenMenu,
              onClose,
            }),
            headerStyle: { backgroundColor: colors.bg },
            headerTintColor: colors.textPrimary,
            headerTitleStyle: { fontFamily: fonts.displaySemi, fontSize: 18 },
            animation: reducedMotion ? "none" : "default",
            headerTitleAlign: "left",
            header: WorkspaceDrawerHeader,
            headerShadowVisible: false,
            headerBackButtonDisplayMode: "minimal",
            contentStyle: { backgroundColor: colors.bg },
          })}
        >
          <SheetStack.Screen
            name="AccountSettings"
            component={AccountSettingsScreen}
            options={{ title: t("Settings") }}
          />
          <SheetStack.Screen
            name="UsageCredits"
            component={UsageCreditsScreen}
            options={{
              title: t("Usage & credits"),
              header: WorkspaceDrawerHeader,
            }}
          />
          <SheetStack.Screen
            name="AccountSecurity"
            component={AccountSecurityScreen}
            options={{ title: t("Account & security") }}
          />
          <SheetStack.Screen
            name="NotificationSettings"
            component={NotificationSettingsScreen}
            options={{ title: t("Notifications") }}
          />
          <SheetStack.Screen
            name="DisplaySettings"
            component={DisplaySettingsScreen}
            options={{ title: t("Display") }}
          />
          <SheetStack.Screen
            name="LanguageSettings"
            component={LanguageSettingsScreen}
            options={{ title: t("Language") }}
          />
          <SheetStack.Screen
            name="VoiceSettings"
            component={VoiceSettingsScreen}
            options={{ title: t("Voice") }}
          />
          <SheetStack.Screen
            name="PrivacySettings"
            component={PrivacySettingsScreen}
            options={{ title: t("Privacy") }}
          />
          <SheetStack.Screen
            name="ProfileEdit"
            component={ProfileFieldEditScreen}
            options={({ route }) => ({
              title: profileFieldTitle(route.params.field),
            })}
          />
          <SheetStack.Screen
            name="SessionDetail"
            component={SessionDetailScreen}
            options={{ title: t("Session") }}
          />
          <SheetStack.Screen
            name="ChatLayoutEdit"
            component={ChatLayoutEditScreen}
            options={({ route }) => ({
              title: chatLayoutTitle(route.params.key),
            })}
          />
          <SheetStack.Screen
            name="TermsOfService"
            component={TermsOfServiceScreen}
            options={{ title: t("Terms of Service") }}
          />
          <SheetStack.Screen
            name="PrivacyPolicy"
            component={PrivacyPolicyScreen}
            options={{ title: t("Privacy Policy") }}
          />
          <SheetStack.Screen
            name="DeviceLogin"
            component={DeviceLoginScreen}
            options={{ title: t("Scan QR Code") }}
          />
          <SheetStack.Screen
            name="AgentKeyLogin"
            component={AgentKeyLoginScreen}
            // Minting a terminal code says so, matching the drawer row.
            options={({ route }) => ({
              title: route.params?.mint
                ? t("Terminal Login Code")
                : t("Agent Key Login"),
            })}
          />
        </SheetStack.Navigator>
      </WorkspaceSheetSurface>
    </WorkspaceSheetContext.Provider>
  );
}

/** Canonical auth/legal URLs share the drawer after sign-in. */
function SheetEntryRoute({
  navigation,
  route,
}: NativeStackScreenProps<
  RootStackParamList,
  "DeviceLogin" | "AgentKeyLogin" | "TermsOfService" | "PrivacyPolicy"
>) {
  useLayoutEffect(() => {
    const action = workspaceSharedEntryNavigationAction(
      navigation.getState(),
      route.name,
      route.params,
    );
    if (action) navigation.dispatch(action);
  }, [navigation, route.name, route.params]);
  return null;
}

export function AppNavigator({
  onOpenMenu,
  onNavigationGuardChange,
  onEdgeBackChange,
  onNavigationCancelled,
  onWorkspacePagePainted,
  suppressWorkspaceAnimation = false,
  renderWorkspace,
}: {
  onOpenMenu: () => void;
  onNavigationGuardChange: WorkspaceNavigationGuardChange;
  onEdgeBackChange: WorkspaceEdgeBackChange;
  onNavigationCancelled: () => void;
  onWorkspacePagePainted: (identity: string) => void;
  suppressWorkspaceAnimation?: boolean;
  renderWorkspace: (children: React.ReactNode) => React.ReactElement;
}) {
  const { t } = useAppLanguage();
  const { isAuthenticated, isRestoring } = useAuthSession();
  const reducedMotion = useReducedMotion();
  const { colors } = useTheme();
  if (isRestoring)
    return (
      <FullScreenLoading
        title={t("Signing you in…")}
        subtitle="Restoring your secure session"
      />
    );
  return (
    <WorkspaceSessionContext.Provider value={isAuthenticated}>
      <WorkspaceMenuContext.Provider value={onOpenMenu}>
        <WorkspaceNavigationGuardContext.Provider
          value={onNavigationGuardChange}
        >
          <WorkspaceEdgeBackContext.Provider value={onEdgeBackChange}>
            <WorkspaceNavigationCancelledContext.Provider
              value={onNavigationCancelled}
            >
              <WorkspacePagePaintedContext.Provider
                value={onWorkspacePagePainted}
              >
                <WorkspaceRootSwapContext.Provider
                  value={suppressWorkspaceAnimation}
                >
                  <Stack.Navigator
                    initialRouteName={isAuthenticated ? "Workspace" : "Auth"}
                    screenOptions={{
                      headerShown: false,
                      headerStyle: { backgroundColor: colors.bg },
                      headerTintColor: colors.textPrimary,
                      headerTitleStyle: {
                        fontFamily: fonts.displaySemi,
                        fontSize: 18,
                      },
                      headerShadowVisible: false,
                      contentStyle: { backgroundColor: colors.bg },
                      headerBackButtonDisplayMode: "minimal",
                    }}
                  >
                    {isAuthenticated ? (
                      <Stack.Group navigationKey="workspace">
                        <Stack.Screen
                          name="Workspace"
                          component={WorkspaceNavigator}
                          layout={({ children }) => renderWorkspace(children)}
                        />
                        <Stack.Screen
                          name="WorkspaceSheet"
                          component={SettingsSheetRoute}
                          options={{
                            presentation: "transparentModal",
                            // The surface animates its panel and stationary scrim independently.
                            animation: "none",
                            gestureEnabled: false,
                            headerShown: false,
                            contentStyle: { backgroundColor: "transparent" },
                          }}
                        />
                        <Stack.Screen
                          name="MachineDesktop"
                          component={MachineDesktopRoute}
                          options={{
                            presentation: "fullScreenModal",
                            animation: reducedMotion ? "none" : "fade",
                            headerShown: false,
                            gestureEnabled: false,
                          }}
                        />
                      </Stack.Group>
                    ) : (
                      <Stack.Screen
                        name="Auth"
                        navigationKey="auth"
                        component={AuthHomeScreen}
                        options={{ title: t("NyxID Sign In") }}
                      />
                    )}
                    <Stack.Group
                      navigationKey={
                        isAuthenticated ? "signed-in" : "signed-out"
                      }
                    >
                      <Stack.Screen
                        name="TermsOfService"
                        component={
                          isAuthenticated
                            ? SheetEntryRoute
                            : TermsOfServiceScreen
                        }
                        options={{ title: t("Terms of Service") }}
                      />
                      <Stack.Screen
                        name="PrivacyPolicy"
                        component={
                          isAuthenticated
                            ? SheetEntryRoute
                            : PrivacyPolicyScreen
                        }
                        options={{ title: t("Privacy Policy") }}
                      />
                      <Stack.Screen
                        name="DeviceLogin"
                        component={
                          isAuthenticated ? SheetEntryRoute : DeviceLoginScreen
                        }
                        options={{ title: t("Approve Device Login") }}
                      />
                      <Stack.Screen
                        name="AgentKeyLogin"
                        component={
                          isAuthenticated
                            ? SheetEntryRoute
                            : AgentKeyLoginScreen
                        }
                        options={{ title: t("Agent Key Login") }}
                      />
                    </Stack.Group>
                  </Stack.Navigator>
                </WorkspaceRootSwapContext.Provider>
              </WorkspacePagePaintedContext.Provider>
            </WorkspaceNavigationCancelledContext.Provider>
          </WorkspaceEdgeBackContext.Provider>
        </WorkspaceNavigationGuardContext.Provider>
      </WorkspaceMenuContext.Provider>
    </WorkspaceSessionContext.Provider>
  );
}
