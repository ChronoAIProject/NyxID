import assert from "node:assert/strict";
import {
  CommonActions,
  StackRouter,
  type NavigationAction,
  type NavigationState,
  type PartialState,
} from "@react-navigation/routers";
import { hydratedTestRouter } from "./routerTestHarness";

export const contentNames = [
  "Assistant",
  "AssistantContext",
  "AssistantDetails",
  "AssistantHistory",
  "AssistantWorkingContext",
  "GroupChat",
  "Automations",
  "Agents",
  "Plugins",
  "Services",
  "Machines",
  "Activity",
  "WorkspaceSearch",
  "WorkspaceRecents",
  "AgentCreate",
  "AgentDetail",
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
  "ActiveGrants",
  "MachinePolicy",
  "MachineActivity",
  "AutomationDetail",
  "AutomationRuns",
  "ActivityDetail",
];
export const settingsNames = [
  "AccountSettings",
  "AccountSecurity",
  "UsageCredits",
  "DeviceLogin",
  "AgentKeyLogin",
  "TermsOfService",
  "PrivacyPolicy",
];
export const rootNames = [
  "Workspace",
  "WorkspaceSheet",
  "MachineDesktop",
  "Auth",
  "DeviceLogin",
  "AgentKeyLogin",
  "TermsOfService",
  "PrivacyPolicy",
];
const options = (routeNames: string[]) => ({
  routeNames,
  routeParamList: {},
  routeGetIdList: {},
});
export const contentOptions = options(contentNames);
export const settingsOptions = options(settingsNames);
export const rootOptions = options(rootNames);
export const contentRouter = hydratedTestRouter(
  StackRouter({ initialRouteName: "Assistant" }),
);
export const settingsRouter = hydratedTestRouter(
  StackRouter({ initialRouteName: "AccountSettings" }),
);
export const rootRouter = hydratedTestRouter(
  StackRouter({ initialRouteName: "Workspace" }),
);

export function hydrate(
  state: NavigationState | PartialState<NavigationState>,
): NavigationState {
  const root = rootRouter.getRehydratedState(state, rootOptions);
  return {
    ...root,
    routes: root.routes.map((route) => {
      if (route.name !== "Workspace" && route.name !== "WorkspaceSheet")
        return route;
      const router =
        route.name === "Workspace" ? contentRouter : settingsRouter;
      const config =
        route.name === "Workspace" ? contentOptions : settingsOptions;
      let child = route.state
        ? router.getRehydratedState(route.state, config)
        : router.getInitialState(config);
      const pending = route.params as
        | { screen?: string; params?: object; merge?: boolean; pop?: boolean }
        | undefined;
      if (pending?.screen) {
        child = router.getStateForAction(
          child,
          CommonActions.navigate({
            name: pending.screen,
            params: pending.params,
            merge: pending.merge,
            pop: pending.pop,
          }),
          config,
        )!;
        assert.ok(child, `Child rejected ${pending.screen}`);
      }
      return { ...route, params: undefined, state: child };
    }),
  };
}
export function fixture() {
  return hydrate({
    routes: [
      {
        name: "Workspace",
        state: {
          routes: [
            {
              name: "Assistant",
              params: {
                home: false,
                agentId: "base-agent",
                conversationId: "base-chat",
                draftId: "saved-draft",
              },
            },
          ],
        },
      },
    ],
  });
}
export function content(root: NavigationState): NavigationState {
  return root.routes.find((route) => route.name === "Workspace")!
    .state as NavigationState;
}
export function selected(
  state: NavigationState | PartialState<NavigationState>,
) {
  return state.routes[state.index ?? state.routes.length - 1]!;
}
export function apply(
  root: NavigationState,
  action: NavigationAction | null,
): NavigationState {
  assert.ok(action);
  if (action.target && action.target === content(root)?.key) {
    const child = contentRouter.getStateForAction(
      content(root),
      action,
      contentOptions,
    );
    assert.ok(child);
    return {
      ...root,
      routes: root.routes.map((route) =>
        route.name === "Workspace" ? { ...route, state: child } : route,
      ),
    };
  }
  const next = rootRouter.getStateForAction(root, action, rootOptions);
  assert.ok(next);
  return hydrate(next);
}
export function pushContent(
  root: NavigationState,
  name: string,
  params?: object,
) {
  const child = content(root);
  return apply(root, {
    type: "PUSH",
    target: child.key,
    payload: { name, params },
  });
}
export function backContent(root: NavigationState) {
  return apply(root, { ...CommonActions.goBack(), target: content(root).key });
}
