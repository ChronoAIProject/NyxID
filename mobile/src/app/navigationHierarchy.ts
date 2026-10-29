import {
  CommonActions,
  StackActions,
  type NavigationState,
  type PartialState,
} from "@react-navigation/routers";
import type {
  WorkspaceContentParamList,
  WorkspaceParamList,
  WorkspaceSheetParamList,
} from "./navigationTypes";
import {
  navigationSnapshot,
  stackStateWithRoutes,
  type WorkspaceState,
} from "./navigationState";
import {
  workspaceContentRouteNames,
  workspaceSettingsRouteNames,
} from "./navigationTypes";
import { assistantHomeParams } from "./assistantChatParams";

export const workspaceRouterOptions = {
  initialRouteName: "Assistant",
} as const;

/** Settings is the only navigation sheet; account/quick-action pages stay inside it. */
export function openWorkspaceSheetAction<
  Name extends keyof WorkspaceSheetParamList,
>(name: Name, params?: WorkspaceSheetParamList[Name]) {
  return {
    type: "NAVIGATE" as const,
    target: undefined as string | undefined,
    payload: {
      name: "WorkspaceSheet",
      params: { screen: name, params, initial: true, merge: true, pop: true },
      merge: true,
    },
  };
}

/** Main pages always resolve inside the slab, including calls from account pages. */
export function openWorkspacePageAction<
  Name extends keyof WorkspaceContentParamList,
>(name: Name, params?: WorkspaceContentParamList[Name]) {
  return {
    type: "NAVIGATE" as const,
    target: undefined as string | undefined,
    payload: {
      name: "Workspace",
      params: { screen: name, params, initial: false, merge: true, pop: true },
      merge: true,
      pop: true,
    },
  };
}

export type WorkspaceSharedEntryName =
  | "DeviceLogin"
  | "AgentKeyLogin"
  | "TermsOfService"
  | "PrivacyPolicy";
export function workspaceSharedEntryNavigationAction<
  Name extends WorkspaceSharedEntryName,
>(
  state: NavigationState | undefined,
  name: Name,
  params?: WorkspaceSheetParamList[Name],
) {
  if (!state?.routeNames.includes("WorkspaceSheet")) return null;
  const entry = state.routes[state.index];
  if (!entry || entry.name !== name) return null;
  const existing = state.routes.find(
    (route) => route.name === "WorkspaceSheet",
  );
  if (!existing)
    return {
      ...StackActions.replace("WorkspaceSheet", {
        screen: name,
        params,
        initial: true,
      }),
      source: entry.key,
      target: state.key,
    };
  const pending = existing.params as
    | {
        screen?: string;
        params?: object;
        state?: NavigationState | PartialState<NavigationState>;
      }
    | undefined;
  const previous = existing.state
    ? navigationSnapshot(existing.state)
    : pending?.state
      ? navigationSnapshot(pending.state)
      : undefined;
  const sheetRoutes = previous?.routes ?? [
    { name: "AccountSettings" },
    ...(pending?.screen && pending.screen !== "AccountSettings"
      ? [{ name: pending.screen, params: pending.params }]
      : []),
  ];
  const sheet = {
    ...existing,
    params: undefined,
    state: stackStateWithRoutes(
      previous,
      [...sheetRoutes, { key: entry.key, name, params }],
      workspaceSettingsRouteNames,
    ),
  };
  const routes = state.routes
    .filter((route) => route !== entry)
    .map((route) => (route === existing ? sheet : route));
  return {
    ...CommonActions.reset({ ...state, index: routes.indexOf(sheet), routes }),
    target: state.key,
  };
}

/** The exact mounted content route remains beneath Settings. */
export function dismissWorkspaceSheetAction(
  state: NavigationState | undefined,
) {
  if (!state?.routes.some((route) => route.name === "Workspace")) return null;
  const routes = state.routes.filter(
    (route) => route.name !== "WorkspaceSheet",
  );
  if (routes.length === state.routes.length) return null;
  return {
    ...CommonActions.reset({ ...state, index: routes.length - 1, routes }),
    target: state.key,
  };
}

/** Sidebar destinations replace obsolete content history, retaining its chat key. */
export function workspaceNavigateAction<
  Name extends keyof WorkspaceContentParamList | "AccountSettings",
>(
  name: Name,
  params?: Name extends keyof WorkspaceContentParamList
    ? WorkspaceContentParamList[Name]
    : WorkspaceParamList["AccountSettings"],
  state?: NavigationState,
) {
  if (name === "AccountSettings")
    return openWorkspaceSheetAction(
      "AccountSettings",
      params as WorkspaceParamList["AccountSettings"],
    );
  if (!state) return openWorkspacePageAction(name, params as never);
  const workspace = state.routes.find((route) => route.name === "Workspace");
  if (!workspace) return openWorkspacePageAction(name, params as never);
  const content = workspace.state
    ? navigationSnapshot(workspace.state)
    : undefined;
  const chat = content?.routes.find((route) => route.name === "Assistant") ?? {
    name: "Assistant",
    params: assistantHomeParams(),
  };
  const previous = content?.routes.find((route) => route.name === name);
  const routes =
    name === "Assistant"
      ? [{ ...chat, params }]
      : [chat, { ...previous, name, params }];
  return {
    ...CommonActions.reset({
      ...state,
      index: 0,
      routes: [
        {
          ...workspace,
          params: undefined,
          state: stackStateWithRoutes(
            content,
            routes,
            workspaceContentRouteNames,
          ),
        },
      ],
    }),
    target: state.key,
  };
}

export function challengeNavigationAction(
  state: NavigationState | undefined,
  challengeId: string,
  blocked = false,
) {
  if (blocked || !state?.routeNames.includes("Workspace")) return null;
  return workspaceNavigateAction("Activity", { challengeId }, state);
}

const contentParent = (name: string) =>
  name.startsWith("Machine")
    ? "Machines"
    : name === "ActiveGrants"
      ? "Activity"
      : name === "PluginConnections"
        ? "Plugins"
        : name.startsWith("Automation")
          ? "Automations"
          : name === "ActivityDetail"
            ? "Activity"
            : name === "ServiceSetup"
              ? "Plugins"
              : [
                    "AgentDetail",
                    "AgentCreate",
                    "AgentMachines",
                    "AgentMachineEditor",
                    "AgentSkills",
                    "GroupEditor",
                  ].includes(name)
                ? "Agents"
                : undefined;

/** Cold links get Home and a module root; Settings keeps that slab underneath. */
export function withWorkspaceFallback(
  state: WorkspaceState | undefined,
): PartialState<NavigationState> | undefined {
  if (!state) return state;
  const snapshot = navigationSnapshot(state);
  let routes = snapshot.routes.map((route) => {
    if (route.name === "Workspace") {
      let pages = route.state?.routes ?? [
        { name: "Assistant", params: assistantHomeParams() },
      ];
      const active = pages[route.state?.index ?? pages.length - 1];
      const parent = active ? contentParent(active.name) : undefined;
      if (parent && !pages.some((page) => page.name === parent))
        pages = [
          ...pages.slice(0, -1),
          {
            name: parent,
            ...(active?.name === "GroupEditor"
              ? { params: { tab: "groups" } }
              : {}),
          },
          pages.at(-1)!,
        ];
      if (!pages.some((page) => page.name === "Assistant"))
        pages = [
          { name: "Assistant", params: assistantHomeParams() },
          ...pages,
        ];
      return {
        ...route,
        state: { ...route.state, index: pages.length - 1, routes: pages },
      };
    }
    if (route.name === "WorkspaceSheet" && route.state) {
      let pages = route.state.routes;
      if (!pages.some((page) => page.name === "AccountSettings"))
        pages = [{ name: "AccountSettings" }, ...pages];
      return {
        ...route,
        state: { ...route.state, index: pages.length - 1, routes: pages },
      };
    }
    return route;
  });
  if (
    !routes.some((route) => route.name === "Workspace") &&
    routes.some(
      (route) =>
        route.name === "WorkspaceSheet" || route.name === "MachineDesktop",
    )
  )
    routes = [
      {
        name: "Workspace",
        state: {
          routes: [{ name: "Assistant", params: assistantHomeParams() }],
        },
      },
      ...routes,
    ];
  return { ...snapshot, index: routes.length - 1, routes };
}

/** Native nested pages own Back swipes; root pages expose the base sidebar. */
export function isWorkspaceSidebarRoot(name: string | undefined) {
  return (
    !!name &&
    [
      "Assistant",
      "Automations",
      "Machines",
      "Agents",
      "Plugins",
      "Services",
      "Activity",
      "WorkspaceSearch",
      "WorkspaceRecents",
    ].includes(name)
  );
}

/** Confirm the requested root swap committed (guards can defer/cancel it). */
export function workspaceDestinationMatches(
  state: NavigationState | undefined,
  action: { type: string; payload?: unknown },
) {
  const wanted = action.payload as PartialState<NavigationState> | undefined;
  if (action.type !== "RESET" || !state || !wanted?.routes) return false;
  const leaf = (root: NavigationState | PartialState<NavigationState>) => {
    let route = root.routes[root.index ?? root.routes.length - 1];
    while (route?.state) {
      const nested = route.state;
      route = nested.routes[nested.index ?? nested.routes.length - 1];
    }
    return route;
  };
  const expected = leaf(wanted);
  const actual = leaf(state);
  return (
    actual?.name === expected?.name &&
    Object.entries(expected?.params ?? {}).every(([key, value]) =>
      Object.is(
        (actual?.params as Record<string, unknown> | undefined)?.[key],
        value,
      ),
    )
  );
}

/** Track the actual slab page independently from a Settings route above it. */
export function workspaceContentIdentity(state: NavigationState | undefined) {
  const workspace = state?.routes.find((route) => route.name === "Workspace");
  const content = workspace?.state;
  const page = content?.routes[content.index ?? content.routes.length - 1];
  return JSON.stringify([page?.key, page?.name, page?.params]);
}
