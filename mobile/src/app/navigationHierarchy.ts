import { CommonActions, StackActions } from "@react-navigation/routers";
import type { NavigationState, PartialState } from "@react-navigation/routers";
import type { WorkspaceParamList } from "./navigationTypes";

export const workspaceRouterOptions = {
  initialRouteName: "Assistant",
  backBehavior: "none",
} as const;

/** Drawer selection keeps the destination's existing chat/filter parameters. */
export function workspaceNavigateAction<Name extends keyof WorkspaceParamList>(
  name: Name,
  params?: WorkspaceParamList[Name],
) {
  return CommonActions.navigate({ name, params, merge: true });
}

/** Keep notification taps pending until the signed-in navigator is mounted. */
export function challengeNavigationAction(
  state: NavigationState | undefined,
  challengeId: string,
  blocked = false,
) {
  if (blocked || !state?.routeNames.includes("Workspace")) return null;
  return {
    ...StackActions.popTo(
      "Workspace",
      { screen: "Activity", params: { challengeId } },
      { merge: true },
    ),
    target: state.key,
  };
}

/** A cold detail link has a real parent destination before its Back button. */
export function withWorkspaceFallback(
  state: PartialState<NavigationState> | undefined,
) {
  if (!state) return state;
  const detail = state.routes[state.index ?? state.routes.length - 1];
  const parent = state.routes.find((route) => route.name === "Workspace");
  if (!parent || parent.state || detail?.name === "Workspace") return state;
  const screen =
    detail?.name === "AgentDetail" ||
    detail?.name === "GroupChat" ||
    detail?.name === "GroupEditor"
      ? "Agents"
      : detail?.name === "AutomationDetail" ||
          detail?.name === "AutomationEditor" ||
          detail?.name === "AutomationRuns" ||
          detail?.name === "AutomationPreferences"
        ? "Automations"
        : detail?.name === "AccountSecurity" ||
            detail?.name === "TermsOfService" ||
            detail?.name === "PrivacyPolicy"
          ? "AccountSettings"
          : "Assistant";
  return {
    ...state,
    routes: state.routes.map((route) =>
      route === parent
        ? {
            ...route,
            state: {
              routes: [
                {
                  name: screen,
                  ...(detail?.name === "GroupChat" ||
                  detail?.name === "GroupEditor"
                    ? { params: { tab: "groups" } }
                    : {}),
                },
              ],
            },
          }
        : route,
    ),
  };
}
