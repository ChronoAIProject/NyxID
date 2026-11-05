import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";

type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
function setup(page: "details" | "context" | "title" = "details") {
  let epoch = 7,
    active = true,
    guard: any;
  const actions: any[] = [],
    writes: any[] = [],
    alerts: any[] = [],
    accessChanges: any[] = [],
    invalidations: any[] = [];
  const agent = {
    id: "specialist",
    name: "Research",
    kind: "specialist",
    can_use: true,
    status: "idle",
    access_mode_override: null as string | null,
  };
  let history: any = {
    conversation: {
      id: "nyxa-thread",
      title: "Saved title",
      agent,
      active_turn: null,
    },
  };
  const rename = deferred<any>(),
    remove = deferred<void>();
  let route = { params: { conversationId: "nyxa-thread", agentId: agent.id } };
  const navigation = {
    goBack: () => actions.push(["back"]),
    push: (name: string, params: any) => actions.push(["push", name, params]),
    getParent: () => ({ getState: () => ({}) }),
    dispatch: (a: any) => actions.push(["dispatch", a]),
  };
  const runtime = () => ({
    epoch,
    active,
    online: active,
    available: () => active,
  });
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "react-native": { Alert: { alert: (...a: any[]) => alerts.push(a) } },
    "@tanstack/react-query": {
      useQuery: (o: any) =>
        o.queryKey[0] === "assistant"
          ? {
              data: history,
              refetch: async () => {
                throw new Error("offline after save");
              },
            }
          : { data: { agents: [agent] }, refetch: async () => {} },
      useQueryClient: () => ({
        cancelQueries: async () => {},
        setQueryData: (_key: any, update: any) => {
          history = update(history);
        },
        invalidateQueries: async (options: any) => invalidations.push(options),
      }),
    },
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: (o: any) => {
        guard = o;
      },
    },
    "../../app/workspaceChatNavigation": {
      workspaceChatNavigationAction: (_: any, target: any) => ({ target }),
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["Label", "Notice", "WorkspacePage"].map((s) => [s, s]),
    ),
    "../../components/settings/SettingsList": Object.fromEntries(
      [
        "SettingsGroup",
        "SettingsMenuRow",
        "SettingsPage",
        "SettingsRow",
        "SettingsTextField",
      ].map((s) => [s, s]),
    ),
    "../../components/settings/SettingsHeaderButton": {
      SettingsHeaderButton: "SettingsHeaderButton",
    },
    "../../components/AgentAvatar": { AgentAvatar: "AgentAvatar" },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "../../lib/api/assistantApi": {
      assistantApi: {
        rename: (id: string, title: string) => {
          writes.push(["rename", id, title]);
          return rename.promise;
        },
        delete: (id: string) => {
          writes.push(["delete", id]);
          return remove.promise;
        },
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../groups/useGroupRuntime": { useGroupAvailability: runtime },
    "./useComposerAccessChange": {
      COMPOSER_ACCESS_MODES: ["autopilot", "nyxmode", "ask_every_time"],
      useComposerAccessChange: (props: any) => ({
        mode: props.agentOverride ?? "nyxmode",
        locked: props.disabled || props.activeTurn || !!props.agentOverride,
        error: undefined,
        loadFailed: false,
        retry: () => {},
        change: async (mode: string) => accessChanges.push(mode),
      }),
    },
    "./AssistantWorkingContext": { AssistantWorkingContext: "WorkingContext" },
  });
  const mod = h.load(
    fileURLToPath(new URL("./AssistantDetailsScreen.tsx", import.meta.url)),
  ) as any;
  h.mount(() =>
    mod[
      page === "context"
        ? "AssistantWorkingContextScreen"
        : page === "title"
          ? "AssistantTitleEditScreen"
          : "AssistantDetailsScreen"
    ]({ navigation, route }),
  );
  const all = () => nodes(h.value());
  const button = (label: string) =>
    all().find(
      (n) =>
        (n.type === "SettingsRow" || n.type === "SettingsMenuRow") &&
        (n.props.accessibilityLabel ?? n.props.label) === label,
    )!;
  const title = () => all().find((n) => n.type === "SettingsTextField")!;
  const save = () => guard.options.headerRight();
  return {
    h,
    all,
    button,
    title,
    save,
    accessChanges,
    agent,
    rename,
    remove,
    actions,
    writes,
    alerts,
    invalidations,
    guard: () => guard,
    changeTitle: (value: string) => {
      title().props.onChangeText(value);
      h.flush();
    },
    offline: () => {
      active = false;
      h.render();
    },
    accountChange: () => {
      epoch++;
      h.render();
    },
    scopeChange: () => {
      route = { params: { conversationId: "nyxa-other", agentId: agent.id } };
      h.render();
    },
    data: () => history,
  };
}

test("Details is a grouped list whose rows push Title, History, Working context and Agent pages", () => {
  const x = setup();
  assert.equal(x.h.value().type, "SettingsPage");
  assert.equal(x.guard().defaultRole, "back");
  assert.equal(x.guard().discardOnClose, undefined);
  const title = x.button("Title");
  assert.equal(title.props.value, "Saved title");
  assert.equal(
    x.all().some((n) => n.type === "SettingsTextField"),
    false,
    "the title is edited on its own page",
  );
  for (const label of ["Title", "Chat history", "Working context", "Research"])
    x.button(label).props.onPress();
  assert.deepEqual(
    x.actions.map((a) => [...a.slice(0, 2), { ...a[2] }]),
    [
      [
        "push",
        "AssistantTitleEdit",
        { conversationId: "nyxa-thread", agentId: "specialist" },
      ],
      ["push", "AssistantHistory", { agentId: "specialist" }],
      ["push", "AssistantWorkingContext", { conversationId: "nyxa-thread" }],
      ["push", "AgentDetail", { agentId: "specialist" }],
    ],
  );
  assert.equal(x.button("Research").props.detail, "Personal specialist");
  assert.equal(
    x.all().some((n) => n.type === "Modal" || n.type === "ActionButton"),
    false,
  );
  x.h.unmount();
});

test("the title page guards navigation and a successful rename updates the cache even if refresh fails", async () => {
  const x = setup("title");
  assert.equal(x.guard().discardOnClose, undefined);
  assert.equal(x.title().props.value, "Saved title");
  assert.equal(x.save().props.enabled, false);
  x.changeTitle("New title");
  assert.equal(x.guard().discardOnClose.title, "Discard title changes?");
  const save = x.save();
  assert.equal(save.props.enabled, true);
  save.props.onPress();
  save.props.onPress();
  x.h.flush();
  assert.equal(x.guard().disabled, true);
  assert.equal(x.save().props.busy, true);
  assert.equal(x.title().props.editable, false);
  assert.deepEqual(x.writes, [["rename", "nyxa-thread", "New title"]]);
  x.rename.resolve({ ...x.data().conversation, title: "New title" });
  await x.h.settle();
  assert.equal(x.data().conversation.title, "New title");
  assert.equal(x.guard().discardOnClose, undefined);
  assert.equal(x.guard().disabled, false);
  assert.equal(x.invalidations.length, 2);
  assert.deepEqual(x.actions, [["back"]], "returns to Chat details once saved");
  x.h.unmount();
});

test("Delete Chat is a solid destructive row behind confirmation, and returns to a fresh chat with the same agent", async () => {
  const x = setup();
  const row = x.button("Delete Chat");
  assert.equal(row.props.destructive, true);
  assert.equal(row.props.centered, true);
  row.props.onPress();
  assert.equal(x.writes.length, 0);
  const confirm = x.alerts[0][2].find((a: any) => a.style === "destructive");
  confirm.onPress();
  x.h.flush();
  assert.equal(x.guard().disabled, true);
  x.remove.resolve();
  await x.h.settle();
  assert.deepEqual(x.writes, [["delete", "nyxa-thread"]]);
  assert.equal(x.actions.length, 1);
  assert.equal(x.actions[0][1].target.kind, "agent");
  assert.equal(x.actions[0][1].target.agentId, "specialist");
  assert.equal(x.actions[0][1].target.conversationId, undefined);
  x.h.unmount();
});

test("permissions are one pop-up menu row, and agent overrides are read-only", () => {
  const x = setup();
  const row = x.button("Permissions");
  assert.equal(row.type, "SettingsMenuRow");
  assert.equal(row.props.value, "nyxmode");
  assert.deepEqual(
    row.props.options.map((o: any) => o.label),
    ["Full access", "Auto", "Ask every time"],
  );
  assert.equal(row.props.disabled, false);
  row.props.onChange("ask_every_time");
  assert.deepEqual(x.accessChanges, ["ask_every_time"]);
  x.agent.access_mode_override = "autopilot";
  x.h.render();
  assert.equal(x.button("Permissions").props.disabled, true);
  assert.equal(x.button("Permissions").props.value, "autopilot");
  const group = x
    .all()
    .find(
      (n) =>
        n.type === "SettingsGroup" &&
        typeof n.props.footer === "string" &&
        n.props.footer.includes("This agent’s override is read-only here."),
    );
  assert.ok(group);
  assert.equal(
    x
      .all()
      .some(
        (n) =>
          typeof n.props.label === "string" && /model/i.test(n.props.label),
      ),
    false,
  );
  x.h.unmount();
});

test("offline callbacks and stale account, scope or unmounted write completions cannot mutate the current page", async () => {
  const offline = setup("title");
  offline.changeTitle("New title");
  const stale = offline.save();
  offline.offline();
  stale.props.onPress();
  assert.equal(offline.writes.length, 0);
  offline.h.unmount();
  for (const change of ["accountChange", "scopeChange", "unmount"] as const) {
    const x = setup("title");
    x.changeTitle("Old account title");
    x.save().props.onPress();
    if (change === "unmount") x.h.unmount();
    else x[change]();
    x.rename.resolve({ ...x.data().conversation, title: "Old account title" });
    await x.h.settle();
    assert.equal(x.data().conversation.title, "Saved title");
    assert.equal(x.invalidations.length, 0);
    assert.equal(x.actions.length, 0);
    if (change !== "unmount") x.h.unmount();
  }
});

test("Working context owns a main-page Back guard while reset is pending", () => {
  const x = setup("context");
  assert.equal(x.h.value().props.title, "Working context");
  const context = x.all().find((n) => n.type === "WorkingContext")!;
  context.props.onBusyChange(true);
  x.h.flush();
  assert.equal(x.guard().disabled, true);
  context.props.onBusyChange(false);
  x.h.flush();
  assert.equal(x.guard().disabled, false);
  context.props.onManageMemory("specialist");
  assert.equal(x.actions[0][1], "AgentDetail");
  x.h.unmount();
});
