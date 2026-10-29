import assert from "node:assert/strict";
import test from "node:test";
import { URL } from "node:url";
import { readFileSync } from "node:fs";
import { getPathFromState, getStateFromPath } from "@react-navigation/core";
import {
  CommonActions,
  StackActions,
  type NavigationState,
} from "@react-navigation/routers";
import { workspaceStateFromPath } from "./workspaceCanonicalLinks";
import { navigationLinkingConfig } from "./navigationLinking";
import { desktopRouteParams } from "../features/machines/desktopEntry";
import {
  isWorkspaceSidebarRoot,
  challengeNavigationAction,
  dismissWorkspaceSheetAction,
  openWorkspacePageAction,
  openWorkspaceSheetAction,
  withWorkspaceFallback,
  workspaceNavigateAction,
  workspaceSharedEntryNavigationAction,
} from "./navigationHierarchy";
import {
  apply,
  backContent,
  content,
  contentNames,
  fixture,
  hydrate,
  pushContent,
  selected,
  settingsNames,
  settingsOptions,
  settingsRouter,
} from "./workspaceNavigationTestHarness";

function linked(path: string) {
  const state = withWorkspaceFallback(
    getStateFromPath(path, navigationLinkingConfig),
  );
  assert.ok(state, path);
  return state;
}
const mainEntries = [
  ["AssistantDetails", { agentId: "agent", conversationId: "thread" }],
  ["AssistantHistory", { agentId: "agent" }],
  ["AssistantWorkingContext", { conversationId: "thread" }],
  ["Automations", undefined],
  ["Agents", { tab: "groups" }],
  ["Plugins", { tab: "skills" }],
  ["Services", undefined],
  ["Machines", undefined],
  ["Activity", { challengeId: "pending" }],
  ["WorkspaceSearch", undefined],
  ["WorkspaceRecents", undefined],
  ["AgentCreate", undefined],
  ["AgentDetail", { agentId: "agent" }],
  ["GroupEditor", { groupId: "crew" }],
  ["ServiceSetup", { catalogSlug: "docs" }],
  ["AgentSkills", { agentId: "agent" }],
  ...(["SkillDetail", "SkillVersions", "SkillHistory"] as const).map(
    (name) =>
      [
        name,
        {
          agentId: "agent",
          skillId: "skill",
          name: "Exact skill",
        },
      ] as const,
  ),
  ...(["AuthoredSkillReview"] as const).map(
    (name) =>
      [
        name,
        {
          agentId: "agent",
          conversationId: "thread",
          acknowledgementId: "ack",
        },
      ] as const,
  ),
  ["AgentMachines", { agentId: "agent" }],
  ["AgentMachineEditor", { agentId: "agent", machineId: "machine" }],
  ["MachineDetail", { machineId: "machine" }],
  ["PluginConnections", { connectorId: "connector" }],
  ["ActiveGrants", undefined],
  ["MachinePolicy", { machineId: "machine", ownerId: "owner" }],
  ["MachineActivity", { machineId: "machine", ownerId: "owner" }],
  ["AutomationDetail", { id: "watch" }],
  ["AutomationRuns", { id: "watch" }],
  ["ActivityDetail", { challengeId: "challenge" }],
] as const;

test("every main destination uses the native content stack and Back returns to its exact caller", () => {
  for (const [name, params] of mainEntries) {
    const caller = pushContent(fixture(), "AssistantContext", {
      conversationId: "caller",
      draftId: "caller-draft",
    });
    const prior = content(caller);
    const next = apply(caller, openWorkspacePageAction(name, params));
    assert.deepEqual(
      next.routes.map((route) => route.name),
      ["Workspace"],
      name,
    );
    assert.equal(content(next).key, prior.key);
    assert.deepEqual(content(next).routes.slice(0, -1), prior.routes);
    assert.equal(selected(content(next)).name, name);
    assert.deepEqual(selected(content(next)).params, params);
    assert.deepEqual(content(backContent(next)).routes, prior.routes);
  }
});

test("sidebar selection removes obsolete editor history and keeps the mounted base chat key and draft", () => {
  for (const [name, params] of mainEntries) {
    const caller = pushContent(
      pushContent(fixture(), "Agents"),
      "AgentDetail",
      { agentId: "old" },
    );
    const base = content(caller).routes[0]!;
    const next = apply(caller, workspaceNavigateAction(name, params, caller));
    assert.deepEqual(
      content(next).routes.map((route) => route.name),
      ["Assistant", name],
    );
    assert.deepEqual(content(next).routes[0], base);
    assert.deepEqual(selected(content(next)).params, params);
    assert.deepEqual(content(backContent(next)).routes, [base]);
  }
});

test("Settings alone uses a sheet, preserving the complete content history on close", () => {
  const caller = pushContent(
    pushContent(fixture(), "Machines"),
    "MachinePolicy",
    { machineId: "machine", ownerId: "owner" },
  );
  for (const name of settingsNames) {
    const next = apply(
      caller,
      openWorkspaceSheetAction(name as "AccountSettings"),
    );
    assert.deepEqual(
      next.routes.map((route) => route.name),
      ["Workspace", "WorkspaceSheet"],
    );
    assert.equal(selected(selected(next).state!).name, name);
    assert.deepEqual(next.routes[0], caller.routes[0]);
    const closed = apply(next, dismissWorkspaceSheetAction(next));
    assert.deepEqual(closed.routes, caller.routes);
  }
  assert.equal(dismissWorkspaceSheetAction(caller), null);
  assert.equal(dismissWorkspaceSheetAction(undefined), null);
});

test("main navigation from a Settings quick action closes that sheet before showing the addressed approval", () => {
  const opened = apply(fixture(), openWorkspaceSheetAction("DeviceLogin"));
  const next = apply(
    opened,
    workspaceNavigateAction("Activity", { challengeId: "challenge" }),
  );
  assert.deepEqual(
    next.routes.map((route) => route.name),
    ["Workspace"],
  );
  assert.equal(selected(content(next)).name, "Activity");
  assert.deepEqual(selected(content(next)).params, {
    challengeId: "challenge",
  });
});

test("module deep links keep Home below the main page and canonical URLs stay unchanged", () => {
  for (const [path, name, params] of [
    [
      "automations?agentId=agent%20one",
      "Automations",
      { agentId: "agent one" },
    ],
    ["agents?tab=groups", "Agents", { tab: "groups" }],
    [
      "plugins?tab=skills&agentId=agent%20one",
      "Plugins",
      { tab: "skills", agentId: "agent one" },
    ],
    ["services", "Services", undefined],
    [
      "activity?challengeId=challenge%2Fone",
      "Activity",
      { challengeId: "challenge/one" },
    ],
    ["assistant/machines", "Machines", undefined],
    [
      "assistant/machines?machine=machine-one",
      "Machines",
      { machine: "machine-one" },
    ],
    ["assistant/machines?tab=logins", "Machines", { tab: "logins" }],
    ["search", "WorkspaceSearch", undefined],
    ["recents", "WorkspaceRecents", undefined],
  ] as const) {
    const state = linked(path);
    assert.deepEqual(
      state.routes.map((route) => route.name),
      ["Workspace"],
    );
    const pages = state.routes[0]!.state!;
    assert.deepEqual(
      pages.routes.map((route) => route.name),
      ["Assistant", name],
    );
    assert.deepEqual(selected(pages).params, params);
    const generated = new URL(
      getPathFromState(state, navigationLinkingConfig),
      "https://native.test",
    );
    const expected = new URL(path!, "https://native.test");
    assert.equal(generated.pathname, expected.pathname);
    assert.deepEqual(
      [...generated.searchParams].sort(),
      [...expected.searchParams].sort(),
    );
  }
  assert.equal(
    getPathFromState(linked("assistant/automations"), navigationLinkingConfig),
    "/automations",
  );
});

test("cold detail links add the appropriate main module and pop horizontally back to it", () => {
  for (const [path, name, parent] of [
    ["agents/agent%20one", "AgentDetail", "Agents"],
    ["groups/edit", "GroupEditor", "Agents"],
    ["groups/edit/crew", "GroupEditor", "Agents"],
    ["automations/watch", "AutomationDetail", "Automations"],
    ["automations/watch/runs", "AutomationRuns", "Automations"],
  ]) {
    const state = linked(path!);
    assert.deepEqual(
      state.routes[0]!.state!.routes.map((route) => route.name),
      ["Assistant", parent, name],
    );
    const root = hydrate(state);
    assert.equal(selected(content(backContent(root))).name, parent);
    const generated = new URL(
      getPathFromState(state, navigationLinkingConfig),
      "https://native.test",
    );
    const expected = new URL(path!, "https://native.test");
    assert.equal(generated.pathname, expected.pathname);
    assert.deepEqual(
      [...generated.searchParams].sort(),
      [...expected.searchParams].sort(),
    );
    if (name === "GroupEditor")
      assert.deepEqual(state.routes[0]!.state!.routes[1]!.params, {
        tab: "groups",
      });
  }
  for (const name of [
    "MachinePolicy",
    "MachineActivity",
    "AgentMachines",
    "AgentSkills",
    "ServiceSetup",
  ]) {
    const state = withWorkspaceFallback({
      routes: [{ name: "Workspace", state: { routes: [{ name }] } }],
    })!;
    assert.deepEqual(
      state.routes[0]!.state!.routes.map((route) => route.name),
      [
        "Assistant",
        name.startsWith("Machine")
          ? "Machines"
          : name === "ServiceSetup"
            ? "Plugins"
            : "Agents",
        name,
      ],
    );
  }
});

test("account links add Settings inside the sheet and keep Home beneath it", () => {
  for (const [path, name] of [
    ["account", "AccountSettings"],
    ["account/security", "AccountSecurity"],
    ["account/usage", "UsageCredits"],
  ]) {
    const state = linked(path!);
    assert.deepEqual(
      state.routes.map((route) => route.name),
      ["Workspace", "WorkspaceSheet"],
    );
    assert.equal(state.routes[0]!.state!.routes[0]!.name, "Assistant");
    const settings = selected(state).state!;
    assert.equal(settings.routes[0]!.name, "AccountSettings");
    assert.equal(selected(settings).name, name);
    const generated = new URL(
      getPathFromState(state, navigationLinkingConfig),
      "https://native.test",
    );
    const expected = new URL(path!, "https://native.test");
    assert.equal(generated.pathname, expected.pathname);
    assert.deepEqual(
      [...generated.searchParams].sort(),
      [...expected.searchParams].sort(),
    );
  }
});

test("private, contextual and crew links resolve inside the same content stack", () => {
  for (const [path, name, params] of [
    [
      "chat?agentId=agent&draft=true&openLatest=false",
      "Assistant",
      { agentId: "agent", draft: true, openLatest: false },
    ],
    [
      "chat/conversation?agentId=agent&acknowledgementId=card",
      "AssistantContext",
      {
        conversationId: "conversation",
        agentId: "agent",
        acknowledgementId: "card",
      },
    ],
    ["groups/crew", "GroupChat", { groupId: "crew" }],
  ] as const) {
    const state = linked(path);
    assert.equal(state.routes.length, 1);
    assert.equal(selected(state).name, "Workspace");
    assert.equal(selected(selected(state).state!).name, name);
    assert.deepEqual(selected(selected(state).state!).params, params);
    assert.deepEqual(
      selected(
        selected(linked(getPathFromState(state, navigationLinkingConfig)))
          .state!,
      ).params,
      params,
    );
  }
});

test("desktop stays full screen outside the slab stack with every selector preserved", () => {
  const state = linked(
    "assistant/machines/node/desktop?conversation_id=nyxa-thread&context_id=context&display=dev",
  );
  assert.deepEqual(
    state.routes.map((route) => route.name),
    ["Workspace", "MachineDesktop"],
  );
  assert.deepEqual(desktopRouteParams(selected(state).params as never), {
    nodeId: "node",
    conversationId: "nyxa-thread",
    contextId: "context",
    display: "dev",
  });
});

test("notification routing waits for authentication and all pending write guards", () => {
  assert.equal(challengeNavigationAction(undefined, "challenge"), null);
  assert.equal(
    challengeNavigationAction(
      { ...fixture(), routeNames: ["Auth"] },
      "challenge",
    ),
    null,
  );
  const caller = pushContent(fixture(), "GroupEditor", { groupId: "crew" });
  assert.equal(challengeNavigationAction(caller, "challenge", true), null);
  const next = apply(caller, challengeNavigationAction(caller, "challenge"));
  assert.deepEqual(
    content(next).routes.map((route) => route.name),
    ["Assistant", "Activity"],
  );
  assert.deepEqual(content(next).routes[0], content(caller).routes[0]);
  assert.deepEqual(selected(content(next)).params, {
    challengeId: "challenge",
  });
});

test("signed-out auth/legal URLs keep root aliases and signed-in entries move into Settings", () => {
  for (const [path, name, params] of [
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
  ] as const) {
    assert.equal(selected(linked(path)).name, name);
    const caller = pushContent(fixture(), "GroupChat", { groupId: "crew" });
    for (const hasSheet of [true, false]) {
      const base = hasSheet
        ? apply(caller, openWorkspaceSheetAction("AccountSecurity"))
        : caller;
      const entry = apply(base, StackActions.push(name, params));
      const action = workspaceSharedEntryNavigationAction(entry, name, params);
      assert.ok(action);
      const next = apply(entry, action);
      assert.deepEqual(next.routes[0], caller.routes[0]);
      assert.equal(selected(next).name, "WorkspaceSheet");
      const nested = settingsRouter.getRehydratedState(
        selected(next).state!,
        settingsOptions,
      );
      assert.equal(selected(nested).name, name);
      assert.deepEqual(selected(nested).params, params);
      if (hasSheet) {
        assert.equal(selected(next).key, selected(base).key);
        assert.equal(selected(nested).key, selected(entry).key);
        const back = settingsRouter.getStateForAction(
          nested,
          CommonActions.goBack(),
          settingsOptions,
        )!;
        assert.equal(selected(back).name, "AccountSecurity");
      }
      assert.equal(
        workspaceSharedEntryNavigationAction(next, name, params),
        null,
      );
    }
  }
  assert.equal(
    workspaceSharedEntryNavigationAction(undefined, "DeviceLogin"),
    null,
  );
  assert.equal(
    workspaceSharedEntryNavigationAction(fixture(), "DeviceLogin"),
    null,
  );
});

test("production registers all main pages in ContentStack and only account routes in SheetStack", () => {
  const source = readFileSync(
    new URL("./AppNavigator.tsx", import.meta.url),
    "utf8",
  );
  const registered = (stack: string) =>
    [
      ...source.matchAll(
        new RegExp(`<${stack}\\.Screen\\s+name="([^"]+)"`, "g"),
      ),
    ].map((match) => match[1]);
  assert.deepEqual(registered("ContentStack").sort(), [...contentNames].sort());
  assert.deepEqual(registered("SheetStack").sort(), [...settingsNames].sort());
  assert.equal(source.includes("createBottomTabNavigator"), false);
  assert.equal(source.includes('name="Pages"'), false);
});

test("root pages reveal the sidebar while native nested pages and Settings retain Back ownership", () => {
  for (const name of [
    "Assistant",
    "Automations",
    "Machines",
    "Plugins",
    "Agents",
    "Services",
    "Activity",
    "WorkspaceSearch",
    "WorkspaceRecents",
  ])
    assert.equal(isWorkspaceSidebarRoot(name), true, name);
  for (const name of [
    undefined,
    "AssistantContext",
    "AssistantDetails",
    "AssistantHistory",
    "AssistantWorkingContext",
    "GroupChat",
    "AgentDetail",
    "MachinePolicy",
    "UsageCredits",
    "MachineDesktop",
  ])
    assert.equal(isWorkspaceSidebarRoot(name), false, name);
});

test("a shared entry retains an unmounted Settings target and its native Back parent", () => {
  const base = fixture();
  const root = {
    ...base,
    index: 2,
    routes: [
      ...base.routes,
      {
        name: "WorkspaceSheet",
        key: "pending-settings",
        params: { screen: "AccountSecurity" },
      },
      {
        name: "DeviceLogin",
        key: "external-code",
        params: { user_code: "CODE" },
      },
    ],
  };
  const action = workspaceSharedEntryNavigationAction(root, "DeviceLogin", {
    user_code: "CODE",
  });
  assert.ok(action);
  const next = apply(root, action);
  assert.equal(selected(next).key, "pending-settings");
  assert.equal(selected(selected(next).state!).key, "external-code");
  assert.deepEqual(
    selected(next).state!.routes.map((r) => r.name),
    ["AccountSettings", "AccountSecurity", "DeviceLogin"],
  );
  assert.deepEqual(next.routes[0], base.routes[0]);
});

test("removed automation authoring deep links resolve only to read-only pages", () => {
  const id = "12345678-1234-4123-8123-123456789abc";
  for (const path of [
    "automations/edit",
    "automations/preferences",
    "automations?setup=watch",
    "assistant/automations?setup=watch",
  ]) {
    const state = workspaceStateFromPath(path, (value) =>
      getStateFromPath(value, navigationLinkingConfig),
    )!;
    assert.equal(selected(content(hydrate(state))).name, "Automations");
    assert.equal(selected(content(hydrate(state))).params, undefined);
  }
  const state = workspaceStateFromPath(`automations/edit/${id}`, (value) =>
    getStateFromPath(value, navigationLinkingConfig),
  )!;
  assert.equal(selected(content(hydrate(state))).name, "AutomationDetail");
  assert.deepEqual(selected(content(hydrate(state))).params, { id });
});
