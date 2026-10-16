import assert from "node:assert/strict";
import test from "node:test";
import { getPathFromState, getStateFromPath } from "@react-navigation/core";
import {
  CommonActions,
  StackActions,
  StackRouter,
  TabRouter,
  type NavigationState,
  type PartialState,
} from "@react-navigation/routers";
import { navigationLinkingConfig } from "./navigationLinking";
import {
  challengeNavigationAction,
  withWorkspaceFallback,
  workspaceNavigateAction,
  workspaceRouterOptions,
} from "./navigationHierarchy";
import type { WorkspaceParamList } from "./navigationTypes";

const workspaceLinking = navigationLinkingConfig.screens?.Workspace;
assert.ok(workspaceLinking && typeof workspaceLinking === "object");
const workspaceRouteNames = Object.keys(workspaceLinking.screens ?? {});
const workspaceRouter = TabRouter(workspaceRouterOptions);
const workspaceOptions = {
  routeNames: workspaceRouteNames,
  routeParamList: {},
  routeGetIdList: {},
};
const rootRouter = StackRouter({
  initialRouteName: navigationLinkingConfig.initialRouteName,
});
const rootOptions = {
  routeNames: [
    ...Object.keys(navigationLinkingConfig.screens ?? {}),
    "ServiceSetup",
    "AgentSkills",
    "ActivityDetail",
  ],
  routeParamList: {},
  routeGetIdList: {},
};

function linkedState(path: string) {
  const state = withWorkspaceFallback(
    getStateFromPath(path, navigationLinkingConfig),
  );
  assert.ok(state, `Expected a navigation state for ${path}`);
  return state;
}

function focusedRoute(state: NavigationState | PartialState<NavigationState>) {
  const route = state.routes[state.index ?? state.routes.length - 1];
  assert.ok(route);
  return route;
}

function focusedNames(state: NavigationState | PartialState<NavigationState>) {
  const names: string[] = [];
  let next: NavigationState | PartialState<NavigationState> | undefined = state;
  while (next) {
    const route = focusedRoute(next);
    names.push(route.name);
    next = route.state;
  }
  return names;
}

function switchWorkspace(
  state: ReturnType<typeof workspaceRouter.getInitialState>,
  name: keyof WorkspaceParamList,
  params?: object,
) {
  let next = workspaceRouter.getStateForAction(
    state,
    workspaceNavigateAction(name),
    workspaceOptions,
  );
  assert.ok(next);
  if (params) {
    next = workspaceRouter.getStateForAction(
      next,
      CommonActions.setParams(params),
      workspaceOptions,
    );
    assert.ok(next);
  }
  return next;
}

test("workspace destinations are siblings, without an added root stack entry", () => {
  for (const [path, name] of [
    ["chat", "Assistant"],
    ["automations", "Automations"],
    ["agents", "Agents"],
    ["plugins", "Plugins"],
    ["services", "Services"],
    ["activity", "Activity"],
    ["account", "AccountSettings"],
  ] as const) {
    assert.ok(path && name);
    const state = linkedState(path);
    assert.deepEqual(
      state.routes.map((route) => route.name),
      ["Workspace"],
    );
    assert.deepEqual(focusedNames(state), ["Workspace", name]);
    assert.equal(getPathFromState(state, navigationLinkingConfig), `/${path}`);
  }
});

test("contextual chat deep links retain their exact conversation and card", () => {
  const state = linkedState(
    "chat/conversation-1?acknowledgementId=card-1&agentId=agent-1&draft=false&openLatest=true",
  );
  assert.deepEqual(
    state.routes.map((route) => route.name),
    ["Workspace", "AssistantContext"],
  );
  assert.deepEqual(focusedRoute(state).params, {
    conversationId: "conversation-1",
    acknowledgementId: "card-1",
    agentId: "agent-1",
    draft: false,
    openLatest: true,
  });
  const path = getPathFromState(state, navigationLinkingConfig);
  const reopened = linkedState(path);
  assert.deepEqual(focusedNames(reopened), ["AssistantContext"]);
  assert.deepEqual(focusedRoute(reopened).params, focusedRoute(state).params);
});

test("a chat link without an ID opens the workspace conversation", () => {
  const state = linkedState("chat?agentId=agent-1&draft=true&openLatest=false");
  assert.deepEqual(focusedNames(state), ["Workspace", "Assistant"]);
  const workspace = focusedRoute(state).state;
  assert.ok(workspace);
  assert.deepEqual(focusedRoute(workspace).params, {
    agentId: "agent-1",
    draft: true,
    openLatest: false,
  });
});

test("workspace links keep encoded query values on their destination page", () => {
  for (const [path, name, params] of [
    [
      "automations?agentId=agent%20one&setup=4b47953b-92cc-482f-8443-e0bcc30ade21",
      "Automations",
      { agentId: "agent one", setup: "4b47953b-92cc-482f-8443-e0bcc30ade21" },
    ],
    [
      "plugins?tab=skills&agentId=agent%20one",
      "Plugins",
      { tab: "skills", agentId: "agent one" },
    ],
    [
      "activity?challengeId=challenge%2Fone",
      "Activity",
      { challengeId: "challenge/one" },
    ],
  ] as const) {
    const state = linkedState(path);
    assert.deepEqual(focusedNames(state), ["Workspace", name]);
    const workspace = focusedRoute(state).state;
    assert.ok(workspace);
    assert.deepEqual(focusedRoute(workspace).params, params);
  }
});

test("existing security, login and legal links keep their destination and parameters", () => {
  for (const [path, name, params] of [
    ["account/security", "AccountSecurity", undefined],
    [
      "login/device?user_code=ABCD-EFGH",
      "DeviceLogin",
      { user_code: "ABCD-EFGH" },
    ],
    [
      "login/agent-key?user_code=IJKL-MNOP",
      "AgentKeyLogin",
      { user_code: "IJKL-MNOP" },
    ],
    ["terms", "TermsOfService", undefined],
    ["privacy", "PrivacyPolicy", undefined],
    ["auth", "Auth", undefined],
  ] as const) {
    const route = focusedRoute(linkedState(path));
    assert.equal(route.name, name);
    assert.deepEqual(route.params, params);
  }
});

test("repeated workspace switches reuse routes and retain conversation and plugin context", () => {
  let state = workspaceRouter.getInitialState(workspaceOptions);
  assert.equal(focusedRoute(state).name, "Assistant");
  assert.equal(
    workspaceLinking.initialRouteName,
    workspaceRouterOptions.initialRouteName,
  );
  const keys = state.routes.map((route) => route.key);
  const conversation = {
    conversationId: "conversation-1",
    acknowledgementId: "card-1",
    agentId: "agent-1",
    draft: true,
  };
  state = switchWorkspace(state, "Assistant", conversation);
  state = switchWorkspace(state, "Plugins", {
    tab: "skills",
    agentId: "agent-2",
  });
  state = switchWorkspace(state, "Automations", { agentId: "agent-2" });

  for (const name of [
    "Services",
    "Agents",
    "Automations",
    "Activity",
    "AccountSettings",
    "Assistant",
    "Plugins",
    "Services",
    "Assistant",
    "Plugins",
  ] as const) {
    state = switchWorkspace(state, name);
    assert.equal(focusedRoute(state).name, name);
    assert.deepEqual(
      state.routes.map((route) => route.key),
      keys,
    );
    assert.equal(state.routes.length, workspaceRouteNames.length);
    assert.equal(state.history.length, 1);
  }

  assert.deepEqual(
    state.routes.find((route) => route.name === "Assistant")?.params,
    conversation,
  );
  assert.deepEqual(
    state.routes.find((route) => route.name === "Automations")?.params,
    { agentId: "agent-2" },
  );
  assert.deepEqual(
    state.routes.find((route) => route.name === "Plugins")?.params,
    { tab: "skills", agentId: "agent-2" },
  );
  assert.equal(
    workspaceRouter.getStateForAction(
      state,
      CommonActions.goBack(),
      workspaceOptions,
    ),
    null,
  );
});

test("closing nested details restores the calling workspace with its route state", () => {
  let workspace = workspaceRouter.getInitialState(workspaceOptions);
  workspace = switchWorkspace(workspace, "Activity", {
    challengeId: "challenge-1",
  });
  let root = rootRouter.getInitialState(rootOptions);
  root = {
    ...root,
    routes: root.routes.map((route) => ({ ...route, state: workspace })),
  };
  const caller = root.routes[0];

  for (const [name, params] of [
    ["ActivityDetail", { challengeId: "challenge-1" }],
    [
      "AssistantContext",
      { conversationId: "conversation-2", acknowledgementId: "card-2" },
    ],
  ] as const) {
    const pushed = rootRouter.getStateForAction(
      root,
      StackActions.push(name, params),
      rootOptions,
    );
    assert.ok(pushed);
    root = pushed;
  }
  assert.deepEqual(
    root.routes.map((route) => route.name),
    ["Workspace", "ActivityDetail", "AssistantContext"],
  );
  const existingContext = { routes: root.routes, index: root.index };
  assert.equal(withWorkspaceFallback(existingContext), existingContext);
  assert.equal(
    withWorkspaceFallback(existingContext)?.routes[0]?.state,
    workspace,
  );

  const backToDetail = rootRouter.getStateForAction(
    root,
    CommonActions.goBack(),
    rootOptions,
  );
  assert.ok(backToDetail);
  assert.equal(focusedRoute(backToDetail).name, "ActivityDetail");
  const closed = rootRouter.getStateForAction(
    backToDetail,
    CommonActions.goBack(),
    rootOptions,
  );
  assert.ok(closed);
  assert.equal(closed.routes.length, 1);
  assert.equal(closed.routes[0], caller);
  assert.deepEqual(focusedNames(closed), ["Workspace", "Activity"]);
});

test("cold detail links return to their own workspace destination", () => {
  for (const [path, name, parent] of [
    [
      "chat/conversation-1?acknowledgementId=card-1",
      "AssistantContext",
      "Assistant",
    ],
    ["agents/agent%20one", "AgentDetail", "Agents"],
    ["groups/group-1", "GroupChat", "Agents"],
    ["groups/edit", "GroupEditor", "Agents"],
    ["groups/edit/group-1", "GroupEditor", "Agents"],
    ["automations/automation-1", "AutomationDetail", "Automations"],
    ["automations/edit", "AutomationEditor", "Automations"],
    ["automations/edit/automation-1", "AutomationEditor", "Automations"],
    ["automations/automation-1/runs", "AutomationRuns", "Automations"],
    ["automations/preferences", "AutomationPreferences", "Automations"],
    ["account/security", "AccountSecurity", "AccountSettings"],
    ["terms", "TermsOfService", "AccountSettings"],
    ["privacy", "PrivacyPolicy", "AccountSettings"],
  ] as const) {
    const root = rootRouter.getRehydratedState(linkedState(path), rootOptions);
    assert.equal(focusedRoute(root).name, name);
    const back = rootRouter.getStateForAction(
      root,
      CommonActions.goBack(),
      rootOptions,
    );
    assert.ok(back);
    assert.deepEqual(
      back.routes.map((route) => route.name),
      ["Workspace"],
    );
    assert.deepEqual(focusedNames(back), ["Workspace", parent]);
    const workspace = focusedRoute(back).state;
    assert.ok(workspace);
    const hydrated = workspaceRouter.getRehydratedState(
      workspace,
      workspaceOptions,
    );
    assert.equal(focusedRoute(hydrated).name, parent);
    assert.equal(hydrated.routes.length, workspaceRouteNames.length);
  }
});

test("group details return to the Groups tab and settings preserve the calling transcript", () => {
  const linked = linkedState("groups/group-1");
  const parent = linked.routes.find(
    (route) => route.name === "Workspace",
  )?.state;
  assert.ok(parent);
  assert.deepEqual(focusedRoute(parent).params, { tab: "groups" });

  const root = rootRouter.getRehydratedState(linked, rootOptions);
  const caller = focusedRoute(root);
  const editing = rootRouter.getStateForAction(
    root,
    StackActions.push("GroupEditor", { groupId: "group-1" }),
    rootOptions,
  );
  assert.ok(editing);
  const closed = rootRouter.getStateForAction(
    editing,
    CommonActions.goBack(),
    rootOptions,
  );
  assert.ok(closed);
  assert.equal(focusedRoute(closed), caller);
  assert.deepEqual(focusedRoute(closed).params, { groupId: "group-1" });
});

test("the Groups hub tab survives other drawer destinations", () => {
  let state = workspaceRouter.getInitialState(workspaceOptions);
  state = switchWorkspace(state, "Agents", { tab: "groups" });
  const key = focusedRoute(state).key;
  state = switchWorkspace(state, "Assistant");
  state = switchWorkspace(state, "Agents");
  assert.equal(focusedRoute(state).key, key);
  assert.deepEqual(focusedRoute(state).params, { tab: "groups" });
  const linked = linkedState("agents?tab=groups");
  const workspace = focusedRoute(linked).state;
  assert.ok(workspace);
  assert.deepEqual(focusedRoute(workspace).params, { tab: "groups" });
});

test("the Web automation alias resolves to the same native workspace sibling", () => {
  const state = linkedState("assistant/automations");
  assert.deepEqual(focusedNames(state), ["Workspace", "Automations"]);
  assert.equal(
    getPathFromState(state, navigationLinkingConfig),
    "/automations",
  );
});

test("selecting an agent automation filter merges the sibling without retaining setup", () => {
  let state = workspaceRouter.getInitialState(workspaceOptions);
  state = switchWorkspace(state, "Automations", { setup: "previous-watch" });
  const key = focusedRoute(state).key;
  const selected = workspaceRouter.getStateForAction(
    state,
    workspaceNavigateAction("Automations", {
      agentId: "researcher",
      setup: undefined,
    }),
    workspaceOptions,
  );
  assert.ok(selected);
  assert.equal(focusedRoute(selected).key, key);
  assert.deepEqual(focusedRoute(selected).params, {
    agentId: "researcher",
    setup: undefined,
  });
});

test("an editor opened from chat retains the exact conversation underneath", () => {
  let root = rootRouter.getRehydratedState(
    linkedState("chat/conversation-1?acknowledgementId=card-1"),
    rootOptions,
  );
  const caller = focusedRoute(root);
  const pushed = rootRouter.getStateForAction(
    root,
    StackActions.push("AutomationEditor", {
      setup: "4b47953b-92cc-482f-8443-e0bcc30ade21",
      agentId: "researcher",
    }),
    rootOptions,
  );
  assert.ok(pushed);
  assert.equal(focusedRoute(pushed).name, "AutomationEditor");
  const closed = rootRouter.getStateForAction(
    pushed,
    CommonActions.goBack(),
    rootOptions,
  );
  assert.ok(closed);
  assert.equal(focusedRoute(closed), caller);
  assert.deepEqual(focusedRoute(closed).params, {
    conversationId: "conversation-1",
    acknowledgementId: "card-1",
  });
});

test("a pending approval tap replaces a restored detail when no workspace caller exists", () => {
  const root = rootRouter.getRehydratedState(
    { routes: [{ name: "AgentDetail", params: { agentId: "agent-1" } }] },
    rootOptions,
  );
  assert.equal(
    rootRouter.getStateForAction(root, CommonActions.goBack(), rootOptions),
    null,
  );
  const action = challengeNavigationAction(root, "challenge-1");
  assert.ok(action);
  assert.equal(action.target, root.key);
  const fallback = rootRouter.getStateForAction(root, action, rootOptions);
  assert.ok(fallback);
  assert.deepEqual(
    fallback.routes.map((route) => route.name),
    ["Workspace"],
  );
  assert.deepEqual(focusedRoute(fallback).params, {
    screen: "Activity",
    params: { challengeId: "challenge-1" },
  });
});

test("notification taps defer until authentication exposes the workspace navigator", () => {
  assert.equal(challengeNavigationAction(undefined, "challenge-1"), null);
  const authOptions = {
    ...rootOptions,
    routeNames: ["Auth", "DeviceLogin", "AgentKeyLogin"],
  };
  const authRouter = StackRouter({ initialRouteName: "Auth" });
  const authState = authRouter.getInitialState(authOptions);
  assert.equal(challengeNavigationAction(authState, "challenge-1"), null);

  const signedInState = rootRouter.getInitialState(rootOptions);
  const action = challengeNavigationAction(signedInState, "challenge-1");
  assert.ok(action);
  assert.equal(action.target, signedInState.key);
  const opened = rootRouter.getStateForAction(
    signedInState,
    action,
    rootOptions,
  );
  assert.ok(opened);
  assert.equal(opened.routes.length, 1);
  assert.deepEqual(focusedRoute(opened).params, {
    screen: "Activity",
    params: { challengeId: "challenge-1" },
  });
});

test("approval taps wait for all critical guards before removing a contextual caller", () => {
  let root = rootRouter.getInitialState(rootOptions);
  const detail = rootRouter.getStateForAction(
    root,
    StackActions.push("ServiceSetup", { connectionId: "connection-1" }),
    rootOptions,
  );
  assert.ok(detail);
  root = detail;
  const guards = new Set([Symbol("saving"), Symbol("uploading")]);
  const pendingChallengeId = "challenge-after-save";
  const pendingAction = () =>
    challengeNavigationAction(root, pendingChallengeId, guards.size > 0);

  assert.equal(pendingAction(), null);
  assert.deepEqual(
    root.routes.map((route) => route.name),
    ["Workspace", "ServiceSetup"],
  );
  guards.delete([...guards][0]!);
  assert.equal(pendingAction(), null);
  guards.clear();
  const action = pendingAction();
  assert.ok(action);
  const opened = rootRouter.getStateForAction(root, action, rootOptions);
  assert.ok(opened);
  assert.deepEqual(
    opened.routes.map((route) => route.name),
    ["Workspace"],
  );
  assert.deepEqual(focusedRoute(opened).params, {
    screen: "Activity",
    params: { challengeId: pendingChallengeId },
  });
});

test("notification navigation removes contextual screens without duplicating the workspace", () => {
  let workspace = workspaceRouter.getInitialState(workspaceOptions);
  workspace = switchWorkspace(workspace, "Assistant", {
    conversationId: "conversation-1",
  });
  workspace = switchWorkspace(workspace, "Plugins", {
    tab: "skills",
    agentId: "agent-1",
  });
  let root = rootRouter.getInitialState(rootOptions);
  root = {
    ...root,
    routes: root.routes.map((route) => ({ ...route, state: workspace })),
  };
  const workspaceKey = root.routes[0]?.key;
  for (const [name, params] of [
    ["AgentDetail", { agentId: "agent-1" }],
    ["AssistantContext", { conversationId: "conversation-2" }],
  ] as const) {
    const next = rootRouter.getStateForAction(
      root,
      StackActions.push(name, params),
      rootOptions,
    );
    assert.ok(next);
    root = next;
  }

  for (const challengeId of ["challenge-1", "challenge-2", "challenge-2"]) {
    const action = challengeNavigationAction(root, challengeId);
    assert.ok(action);
    assert.equal(action.target, root.key);
    const next = rootRouter.getStateForAction(root, action, rootOptions);
    assert.ok(next);
    root = next;
    assert.deepEqual(
      root.routes.map((route) => route.name),
      ["Workspace"],
    );
    assert.equal(root.routes[0]?.key, workspaceKey);
    assert.equal(root.routes[0]?.state, workspace);
    assert.deepEqual(focusedRoute(root).params, {
      screen: "Activity",
      params: { challengeId },
    });
  }
});

test("workspace fallbacks leave unknown paths and existing workspace state untouched", () => {
  assert.equal(withWorkspaceFallback(undefined), undefined);
  assert.equal(
    withWorkspaceFallback(
      getStateFromPath("unrecognized/path", navigationLinkingConfig),
    ),
    undefined,
  );
  const existing = linkedState("plugins?tab=skills&agentId=agent-1");
  assert.equal(withWorkspaceFallback(existing), existing);
  assert.deepEqual(focusedNames(existing), ["Workspace", "Plugins"]);
});
