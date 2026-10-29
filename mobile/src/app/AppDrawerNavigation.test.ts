import { workspaceAccessibilityState } from "../lib/accessibility/workspaceAccessibility";
import { createWorkspaceFocusState } from "../lib/accessibility/workspaceFocusState";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";
import {
  type NavigationAction,
  type NavigationState,
} from "@react-navigation/routers";
import {
  openWorkspaceSheetAction,
  isWorkspaceSidebarRoot,
  workspaceNavigateAction,
  workspaceDestinationMatches,
  workspaceContentIdentity,
} from "./navigationHierarchy";
import {
  activeWorkspaceChatTarget,
  workspaceChatNavigationAction,
  type WorkspaceChatTarget,
} from "./workspaceChatNavigation";

type Element = { type: unknown; props: Record<string, any> };
type Effect = {
  dependencies?: unknown[];
  cleanup?: () => void;
  run: () => void | (() => void);
};
const same = (a?: unknown[], b?: unknown[]) =>
  !!a && !!b && a.length === b.length && a.every((v, i) => Object.is(v, b[i]));
function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children, root.props.sidebar]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

import {
  fixture,
  apply,
  content,
  pushContent,
} from "./workspaceNavigationTestHarness";
import { dismissWorkspaceSheetAction } from "./navigationHierarchy";
import { workspaceContentAnimation } from "../components/workspace/workspacePushState";

/** Extract actual declarations so App's splash/network/push startup effects never execute. */
function shellSource() {
  const filename = fileURLToPath(new URL("./App.tsx", import.meta.url));
  const source = ts.createSourceFile(
    filename,
    readFileSync(filename, "utf8"),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  return (
    ["getActiveRouteName", "ThemedAppShell"]
      .map((name) => {
        const declaration = source.statements.find(
          (node) => ts.isFunctionDeclaration(node) && node.name?.text === name,
        );
        assert.ok(declaration, `Missing actual App declaration ${name}`);
        return declaration.getText(source);
      })
      .join("\n") + "\nexports.ThemedAppShell = ThemedAppShell;"
  );
}

function setup(deferFade = false) {
  const fades: (() => void)[] = [];
  const sequence: string[] = [];
  let timerId = 0;
  const timers = new Map<number, { callback: () => void; delay: number }>();
  let rootState = fixture();
  let authenticated = true;
  let authEpoch = 0;
  let ready = true;
  let rootReads = 0;
  let cursor = 0;
  let dirty = false;
  let rendered!: Element;
  const slots: unknown[] = [];
  const effects = new Map<number, Effect>();
  let pendingEffects: number[] = [];
  const dispatched: NavigationAction[] = [];
  const focused: number[] = [];
  const scheduled: (() => void)[] = [];
  const scheduledWhileHidden: boolean[] = [];
  const nativeMenuOwner = Symbol("native menu host");
  const focus = createWorkspaceFocusState(
    (target) => focused.push(target),
    (callback) => {
      scheduledWhileHidden.push(find("WorkspacePushShell").props.presented);
      scheduled.push(callback);
    },
    () => authEpoch,
  );
  const react = {
    useRef: <T>(initial: T) => {
      const i = cursor++;
      slots[i] ??= { current: initial };
      return slots[i] as { current: T };
    },
    useState: <T>(initial?: T | (() => T)) => {
      const i = cursor++;
      if (!(i in slots))
        slots[i] =
          typeof initial === "function" ? (initial as () => T)() : initial;
      return [
        slots[i] as T,
        (next: T | ((previous: T) => T)) => {
          const value =
            typeof next === "function"
              ? (next as (previous: T) => T)(slots[i] as T)
              : next;
          if (!Object.is(value, slots[i])) {
            slots[i] = value;
            dirty = true;
          }
        },
      ] as const;
    },
    useCallback: <T>(callback: T, dependencies: unknown[]) => {
      const i = cursor++;
      const previous = slots[i] as
        | { callback: T; dependencies: unknown[] }
        | undefined;
      if (!previous || !same(previous.dependencies, dependencies))
        slots[i] = { callback, dependencies };
      return (slots[i] as { callback: T }).callback;
    },
    useEffect: (run: Effect["run"], dependencies?: unknown[]) => {
      const i = cursor++;
      const previous = effects.get(i);
      if (!previous || !same(previous.dependencies, dependencies)) {
        effects.set(i, { dependencies, run, cleanup: previous?.cleanup });
        pendingEffects.push(i);
      }
    },
  };
  const jsx = (type: unknown, props: Record<string, any>) => ({
    type,
    props:
      type === "AppNavigator"
        ? {
            ...props,
            children: props.renderWorkspace({
              type: "ContentNavigator",
              props: {},
            }),
          }
        : props,
  });
  const exports: { ThemedAppShell?: (props: object) => Element } = {};
  const output = ts.transpileModule(shellSource(), {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
      jsx: ts.JsxEmit.ReactJSX,
    },
  }).outputText;
  runInNewContext(output, {
    exports,
    require: (name: string) => {
      assert.equal(
        name,
        "react/jsx-runtime",
        "only extracted component declarations may load",
      );
      return { jsx, jsxs: jsx, Fragment: "Fragment" };
    },
    ...react,
    setTimeout: (callback: () => void, delay: number) => {
      timers.set(++timerId, { callback, delay });
      return timerId;
    },
    clearTimeout: (id: number) => timers.delete(id),
    useTheme: () => ({ mode: "dark" }),
    useSafeAreaInsets: () => ({ top: 40 }),
    useAuthSession: () => ({ isAuthenticated: authenticated }),
    ...Object.fromEntries(
      [
        "NavigationContainer",
        "StatusBar",
        "AppNavigator",
        "View",
        "ToastOverlay",
        "WorkspaceDrawer",
        "WorkspacePushShell",
      ].map((name) => [name, name]),
    ),
    WorkspaceAccessibilityContext: {
      Provider: "WorkspaceAccessibilityProvider",
    },
    workspaceAccessibilityState,
    workspaceFocus: focus,
    appLinking: {},
    pushToastStyles: { overlay: {} },
    appRootStyles: { fill: { flex: 1 } },
    workspaceNavigateAction,
    workspaceChatNavigationAction,
    openWorkspaceSheetAction,
    isWorkspaceSidebarRoot,
    activeWorkspaceChatTarget,
    workspaceDestinationMatches,
    workspaceContentIdentity,
  });
  const component = exports.ThemedAppShell;
  assert.ok(component);
  const navigationRef = {
    isReady: () => ready,
    getRootState: () => {
      rootReads++;
      return rootState;
    },
    getCurrentRoute: () => ({ name: "Assistant" }),
    dispatch: (action: NavigationAction) => {
      sequence.push("swap");
      dispatched.push(action);
    },
  };
  function flush() {
    let iterations = 0;
    while (dirty || pendingEffects.length) {
      if (++iterations > 30) throw new Error("Shell did not settle");
      if (dirty) {
        dirty = false;
        cursor = 0;
        rendered = component!({
          navigationRef,
          currentRouteName: "Assistant",
          setCurrentRouteName: () => {},
          flushPendingChallengeTapNavigation: () => {},
          onNavigationGuardChange: () => {},
          pushToast: null,
        });
      }
      const pending = pendingEffects;
      pendingEffects = [];
      for (const i of pending) effects.get(i)?.cleanup?.();
      for (const i of pending) {
        const effect = effects.get(i)!;
        effect.cleanup = effect.run() || undefined;
      }
    }
  }
  const find = (name: string) => {
    const element = elements(rendered).find((item) => item.type === name);
    assert.ok(element, `Missing rendered ${name}`);
    return element;
  };
  dirty = true;
  flush();
  find("WorkspacePushShell").props.contentTransition.current = {
    fadeOut: (onHidden: () => void) => {
      sequence.push("fade out");
      if (deferFade) fades.push(onHidden);
      else onHidden();
    },
    reveal: () => sequence.push("fade in"),
  };
  return {
    sequence,
    watchdogs: () => [...timers.values()],
    expireWatchdog: () => {
      const pending = [...timers.values()];
      timers.clear();
      for (const timer of pending) timer.callback();
      flush();
    },
    finishFade: () => fades.shift()?.(),
    dispatched,
    focus,
    focused,
    scheduledWhileHidden,
    flushFocus: () => {
      while (scheduled.length) scheduled.shift()!();
    },
    flush,
    drawer: (): Element => {
      const element = find("WorkspaceDrawer");
      return {
        ...element,
        props: {
          ...element.props,
          onDismiss: () => {
            // The actual push shell releases focus ownership before this shell callback.
            focus.setModalActive(nativeMenuOwner, false);
            find("WorkspacePushShell").props.onDismiss();
          },
        },
      };
    },
    shell: () => find("WorkspacePushShell"),
    navigator: () => find("AppNavigator"),
    commitLast: (paint = true) => {
      rootState = apply(rootState, dispatched.at(-1)!);
      find("NavigationContainer").props.onStateChange();
      flush();
      if (paint) {
        find("AppNavigator").props.onWorkspacePagePainted(
          workspaceContentIdentity(rootState),
        );
        flush();
      }
    },
    paint: (identity = workspaceContentIdentity(rootState)) => {
      find("AppNavigator").props.onWorkspacePagePainted(identity);
      flush();
    },
    open: () => {
      find("AppNavigator").props.onOpenMenu();
      flush();
      focus.setModalActive(nativeMenuOwner, true);
    },
    readyNavigation: () => {
      find("NavigationContainer").props.onReady();
      flush();
    },
    changedNavigation: () => {
      find("NavigationContainer").props.onStateChange();
      flush();
    },
    setState: (state: NavigationState) => {
      rootState = state;
    },
    state: () => rootState,
    rootReads: () => rootReads,
    authenticate: (next: boolean) => {
      if (authenticated !== next) authEpoch++;
      authenticated = next;
      if (!next) focus.setModalActive(nativeMenuOwner, false);
      dirty = true;
      flush();
    },
    readiness: (next: boolean) => {
      ready = next;
    },
    hasDrawer: () =>
      elements(rendered).some((item) => item.type === "WorkspaceDrawer"),
  };
}

// Native routers allocate fresh keys for newly opened pages; retained-key coverage
// lives in the real reducer tests. Compare destination/params/action targets here.
const comparable = (value: unknown): unknown =>
  JSON.parse(
    JSON.stringify(value, (key, item) => (key === "key" ? undefined : item)),
  );

const choices = [
  {
    choose: (d: Element) => d.props.onHome(),
    action: (s: NavigationState) =>
      workspaceChatNavigationAction(s, { kind: "home" }),
  },
  {
    choose: (d: Element) => d.props.onNavigate("Assistant"),
    action: (s: NavigationState) =>
      workspaceChatNavigationAction(s, { kind: "home" }),
  },
  {
    choose: (d: Element) => d.props.onNavigate("Machines"),
    action: (s: NavigationState) =>
      workspaceNavigateAction("Machines", undefined, s),
  },
  {
    choose: (d: Element) =>
      d.props.onOpenChat({
        kind: "agent",
        agentId: "researcher",
        conversationId: "chat-1",
      }),
    action: (s: NavigationState) =>
      workspaceChatNavigationAction(s, {
        kind: "agent",
        agentId: "researcher",
        conversationId: "chat-1",
      }),
  },
  {
    choose: (d: Element) =>
      d.props.onOpenChat({ kind: "group", groupId: "crew-1" }),
    action: (s: NavigationState) =>
      workspaceChatNavigationAction(s, { kind: "group", groupId: "crew-1" }),
  },
  {
    choose: (d: Element) => d.props.onCreateAgent(),
    action: (s: NavigationState) =>
      workspaceNavigateAction("AgentCreate", undefined, s),
  },
  {
    choose: (d: Element) => d.props.onCreateGroup(),
    action: (s: NavigationState) =>
      workspaceNavigateAction("GroupEditor", undefined, s),
  },
  ...(["WorkspaceSearch", "WorkspaceRecents"] as const).map((name) => ({
    choose: (d: Element) => d.props.onBrowse(name),
    action: (s: NavigationState) => workspaceNavigateAction(name, undefined, s),
  })),
];

test("every main destination swaps under the open slab with no inner animation, then requests one close", () => {
  for (const choice of choices) {
    const env = setup();
    env.open();
    const before = env.state();
    assert.equal(
      workspaceContentAnimation(
        false,
        env.navigator().props.suppressWorkspaceAnimation,
      ),
      "none",
    );
    choice.choose(env.drawer());
    env.flush();
    assert.equal(
      env.shell().props.visible,
      true,
      "wait for the guarded root swap to commit",
    );
    assert.deepEqual(
      comparable(env.dispatched),
      comparable([choice.action(before)]),
    );
    env.commitLast(false);
    assert.equal(
      env.shell().props.visible,
      true,
      "root state alone is not a native paint",
    );
    env.paint("stale-page");
    assert.equal(env.shell().props.visible, true);
    env.paint();
    assert.equal(
      env.shell().props.visible,
      false,
      "new page slides in once with the slab",
    );
    assert.equal(
      env.navigator().props.suppressWorkspaceAnimation,
      true,
      "no native stack travel during closing",
    );
    env.drawer().props.onDismiss();
    env.flush();
    assert.equal(env.navigator().props.suppressWorkspaceAnimation, false);
    env.drawer().props.onDismiss();
    assert.equal(env.dispatched.length, 1);
  }
});

test("rapid destination double taps cannot replace the in-flight swap or bounce the slab", () => {
  const env = setup();
  env.open();
  const drawer = env.drawer();
  drawer.props.onNavigate("Machines");
  drawer.props.onNavigate("Automations");
  assert.equal(env.dispatched.length, 1);
  env.commitLast();
  drawer.props.onNavigate("Agents");
  env.shell().props.onOpenChange(true);
  env.flush();
  assert.equal(env.shell().props.visible, false);
  assert.equal(env.dispatched.length, 1);
  env.drawer().props.onDismiss();
  env.flush();
  env.open();
  env.drawer().props.onNavigate("Automations");
  assert.equal(
    env.dispatched.length,
    2,
    "another destination works after settlement",
  );
});

test("Keep editing cancels the pending swap, leaves the slab open and permits another guarded selection", () => {
  const env = setup();
  env.open();
  env.drawer().props.onNavigate("Machines");
  env.changedNavigation();
  assert.equal(
    env.shell().props.visible,
    true,
    "a prevented action does not close navigation",
  );
  env.navigator().props.onNavigationCancelled();
  env.drawer().props.onNavigate("Automations");
  assert.equal(env.dispatched.length, 2);
  env.commitLast();
  assert.equal(env.shell().props.visible, false);
});

test("Discard closes only after the deferred guarded action commits, retaining the thread draft", () => {
  const env = setup();
  const chat = content(env.state()).routes[0]!;
  env.setState(
    pushContent(env.state(), "AgentDetail", { agentId: "dirty-agent" }),
  );
  env.open();
  env.drawer().props.onNavigate("Machines");
  env.changedNavigation();
  assert.equal(env.shell().props.visible, true);
  env.commitLast();
  assert.equal(env.shell().props.visible, false);
  assert.deepEqual(content(env.state()).routes[0], chat);
});

test("selecting an already active root closes once without requiring a navigation event", () => {
  const env = setup(true);
  env.setState(
    apply(
      env.state(),
      workspaceNavigateAction("Machines", undefined, env.state()),
    ),
  );
  env.open();
  env.drawer().props.onNavigate("Machines");
  env.flush();
  assert.equal(env.shell().props.visible, false);
  assert.equal(env.dispatched.length, 0);
  assert.equal(env.sequence.includes("fade out"), false);
  assert.equal(env.watchdogs().length, 0);
  env.finishFade();
  assert.equal(env.dispatched.length, 0);
});

test("sliver close while waiting for a new root paint restores content and ignores late paint", () => {
  const env = setup(true);
  env.open();
  env.drawer().props.onNavigate("Machines");
  env.finishFade();
  env.commitLast(false);
  assert.equal(env.shell().props.visible, true);
  env.shell().props.onOpenChange(false);
  env.flush();
  assert.equal(env.shell().props.visible, false);
  assert.equal(env.sequence.at(-1), "fade in");
  assert.equal(env.watchdogs().length, 0);
  const count = env.sequence.length;
  env.paint();
  env.changedNavigation();
  assert.equal(
    env.sequence.length,
    count,
    "late paint/state cannot rearm a cancelled transition",
  );
  env.drawer().props.onDismiss();
  env.flush();
  assert.equal(env.shell().props.presented, false);
});

test("a close interrupts fade-out and fences its delayed root dispatch", () => {
  const env = setup(true);
  env.open();
  env.drawer().props.onNavigate("Machines");
  env.shell().props.onOpenChange(false);
  env.flush();
  assert.equal(env.shell().props.visible, false);
  assert.equal(env.sequence.at(-1), "fade in");
  env.finishFade();
  assert.equal(env.dispatched.length, 0);
  assert.equal(env.watchdogs().length, 0);
  env.drawer().props.onDismiss();
  env.flush();
  env.open();
  env.drawer().props.onNavigate("Automations");
  env.finishFade();
  env.commitLast();
  assert.equal(env.dispatched.length, 1);
  assert.equal(env.shell().props.visible, false);
});

test("the paint watchdog recovers a lost fade callback or root paint without replaying navigation", () => {
  for (const interrupted of ["fade", "paint"]) {
    const env = setup(true);
    env.open();
    env.drawer().props.onNavigate("Machines");
    env.drawer().props.onNavigate("Automations");
    if (interrupted === "paint") {
      env.finishFade();
      env.commitLast(false);
    }
    const [watchdog] = env.watchdogs();
    assert.equal(watchdog?.delay, 2000);
    env.expireWatchdog();
    assert.equal(env.shell().props.visible, false);
    assert.equal(env.sequence.at(-1), "fade in");
    env.finishFade();
    env.paint();
    assert.equal(env.dispatched.length, interrupted === "paint" ? 1 : 0);
    env.drawer().props.onDismiss();
    env.flush();
    assert.equal(env.shell().props.presented, false);
  }
});

test("normal matching paint cancels the watchdog and unrelated state changes cannot restart the fade", () => {
  const env = setup(true);
  env.open();
  env.drawer().props.onNavigate("Machines");
  env.finishFade();
  env.commitLast();
  assert.equal(env.watchdogs().length, 0);
  env.changedNavigation();
  env.paint();
  assert.equal(env.sequence.filter((step) => step === "fade in").length, 1);
  env.expireWatchdog();
  assert.equal(env.dispatched.length, 1);
});

test("Settings presents over the expanded sidebar and dismissal preserves the exact page and open state", () => {
  const env = setup();
  env.readyNavigation();
  env.open();
  const contentBefore = content(env.state());
  env.drawer().props.onNavigate("AccountSettings");
  env.flush();
  assert.equal(env.shell().props.visible, true);
  env.commitLast();
  assert.equal(env.shell().props.visible, true);
  assert.equal(env.shell().props.obscured, true);
  assert.strictEqual(content(env.state()), contentBefore);
  env.setState(apply(env.state(), dismissWorkspaceSheetAction(env.state())));
  env.changedNavigation();
  assert.equal(env.shell().props.visible, true);
  assert.equal(env.shell().props.presented, true);
  assert.equal(env.shell().props.obscured, false);
  assert.strictEqual(content(env.state()), contentBefore);
});

test("a content destination inside Settings closes the sidebar only after the sheet host is removed", () => {
  const env = setup();
  env.readyNavigation();
  env.open();
  env.drawer().props.onNavigate("AccountSettings");
  env.commitLast();
  assert.equal(env.shell().props.visible, true);
  // The sheet's actual guarded dismissal dispatches this only on exit completion.
  env.setState(
    apply(
      env.state(),
      workspaceNavigateAction("Agents", undefined, env.state()),
    ),
  );
  env.changedNavigation();
  assert.equal(env.shell().props.obscured, false);
  assert.equal(env.shell().props.visible, true);
  env.paint();
  assert.equal(env.shell().props.visible, false);
});

test("the shell owns sidebar AX settlement without a duplicate background exclusion, including sign-out cleanup", () => {
  const env = setup();
  assert.equal(env.shell().props.presented, false);
  env.open();
  assert.equal(env.shell().props.presented, true);
  env.shell().props.onOpenChange(false);
  env.flush();
  assert.equal(env.shell().props.presented, true);
  env.drawer().props.onDismiss();
  env.flush();
  assert.equal(env.shell().props.presented, false);
  env.open();
  env.authenticate(false);
  assert.equal(env.hasDrawer(), false);
  assert.equal(env.shell().props.presented, false);
});

test("plain menu Close restores the live invoking owner only after the visible background commits", () => {
  const env = setup();
  env.readyNavigation();
  const scope = env.state().routes[0]!.key;
  const original = Symbol("invoking header");
  let tag = 10;
  env.focus.register(original, () => tag, true, scope);
  env.focus.invoked(original);
  env.open();
  env.shell().props.onOpenChange(false);
  env.flush();
  env.flushFocus();
  assert.deepEqual(env.focused, []);
  env.drawer().props.onDismiss();
  assert.equal(env.shell().props.presented, true);
  env.flush();
  assert.ok(env.scheduledWhileHidden.length > 0);
  assert.ok(env.scheduledWhileHidden.every((hidden) => !hidden));
  env.focus.release(original);
  env.focus.register(Symbol("remounted header"), () => tag, true, scope);
  tag = 20;
  env.flushFocus();
  assert.deepEqual(env.focused, [20]);
});

test("navigation, auth, scope replacement and reopening cancel stale focus restoration", () => {
  for (const cancel of ["navigation", "auth", "scope", "reopen"]) {
    const env = setup();
    env.readyNavigation();
    const scope = env.state().routes[0]!.key;
    const owner = Symbol("header");
    env.focus.register(owner, () => 10, true, scope);
    env.focus.invoked(owner);
    env.open();
    if (cancel === "navigation") {
      env.drawer().props.onNavigate("Agents");
      env.commitLast();
    } else env.shell().props.onOpenChange(false);
    env.flush();
    env.drawer().props.onDismiss();
    env.flush();
    if (cancel === "auth") env.authenticate(false);
    if (cancel === "scope") {
      env.setState({
        ...env.state(),
        routes: [{ name: "GroupChat", key: "replacement-group" }],
        index: 0,
      });
      env.changedNavigation();
    }
    if (cancel === "reopen") env.open();
    env.flushFocus();
    assert.deepEqual(env.focused, [], cancel);
  }
});

test("sign-out fences stale selection and dismissal callbacks without re-dispatching actions", () => {
  for (const choice of choices) {
    const env = setup();
    env.open();
    const stale = env.drawer();
    choice.choose(stale);
    env.flush();
    const count = env.dispatched.length;
    env.authenticate(false);
    choice.choose(stale);
    stale.props.onDismiss();
    env.authenticate(true);
    env.drawer().props.onDismiss();
    assert.equal(env.dispatched.length, count);
    assert.equal(env.shell().props.visible, false);
  }
});

test("readiness is checked before swapping and unavailable selections are never deferred", () => {
  const env = setup();
  env.open();
  env.readiness(false);
  env.drawer().props.onBrowse("WorkspaceSearch");
  env.flush();
  assert.equal(env.shell().props.visible, true);
  assert.equal(env.dispatched.length, 0);
  env.readiness(true);
  env.drawer().props.onBrowse("WorkspaceRecents");
  env.commitLast();
  assert.equal(env.dispatched.length, 1);
  assert.equal(env.shell().props.visible, false);
});

test("root-swap comparison ignores unrelated state events and matches exact agent/thread/draft parameters", () => {
  const state = fixture();
  const action = workspaceChatNavigationAction(state, {
    kind: "agent",
    agentId: "new-agent",
  })!;
  assert.equal(workspaceDestinationMatches(state, action), false);
  const next = apply(state, action);
  assert.equal(workspaceDestinationMatches(next, action), true);
  const other = apply(
    next,
    workspaceChatNavigationAction(next, {
      kind: "agent",
      agentId: "new-agent",
    }),
  );
  assert.equal(
    workspaceDestinationMatches(other, action),
    false,
    "fresh draft identity differs",
  );
  assert.equal(workspaceContentAnimation(true, false), "none");
  assert.equal(workspaceContentAnimation(false, false), "slide_from_right");
});

test("sidebar Quick actions present above the unchanged expanded base", () => {
  for (const action of ["scanner", "terminal"]) {
    const env = setup();
    env.readyNavigation();
    env.open();
    const before = content(env.state());
    env.drawer().props.onQuickAction(action);
    env.commitLast();
    assert.equal(env.shell().props.visible, true);
    assert.equal(env.shell().props.obscured, true);
    assert.strictEqual(content(env.state()), before);
    const sheet = env
      .state()
      .routes.find((r) => r.name === "WorkspaceSheet")!.state!;
    const page = sheet.routes[sheet.index ?? sheet.routes.length - 1]!;
    assert.equal(
      page.name,
      action === "scanner" ? "DeviceLogin" : "AgentKeyLogin",
    );
    assert.deepEqual(
      structuredClone(page.params),
      action === "scanner" ? { start_scanner: true } : { mint: true },
    );
    env.setState(apply(env.state(), dismissWorkspaceSheetAction(env.state())));
    env.changedNavigation();
    assert.equal(env.shell().props.visible, true);
  }
});

test("destination fades old root before the animation:none swap and reveals only its matching paint", () => {
  const env = setup(true);
  env.open();
  env.drawer().props.onNavigate("Machines");
  env.drawer().props.onNavigate("Agents");
  assert.deepEqual(env.sequence, ["fade out"]);
  assert.equal(env.dispatched.length, 0);
  assert.equal(env.shell().props.visible, true);
  env.finishFade();
  assert.deepEqual(env.sequence, ["fade out", "swap"]);
  assert.equal(
    workspaceContentAnimation(
      false,
      env.navigator().props.suppressWorkspaceAnimation,
    ),
    "none",
  );
  env.commitLast(false);
  env.paint("stale");
  assert.deepEqual(env.sequence, ["fade out", "swap"]);
  env.paint();
  assert.deepEqual(env.sequence, ["fade out", "swap", "fade in"]);
  assert.equal(env.shell().props.visible, false);
  env.paint();
  assert.equal(env.sequence.filter((step) => step === "fade in").length, 1);
});
