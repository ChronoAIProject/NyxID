import { newAssistantChatParams } from "./assistantChatParams";
import { translateApp } from "../i18n/appTranslation";
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
  type PartialState,
} from "@react-navigation/routers";
import {
  openScannerSheetAction,
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
  hydrate,
  apply,
  content,
  pushContent,
} from "./workspaceNavigationTestHarness";
import { workspaceCornerSensor } from "../components/workspace/workspaceLayout";
import { dismissWorkspaceSheetAction } from "./navigationHierarchy";
import {
  workspaceContentAnimation,
  workspaceSidebarLayout,
} from "../components/workspace/workspacePushState";
import {
  deferred,
  hookHarness,
} from "../features/assistant/assistantHookTestHarness";
import { createSessionCoordinator } from "../lib/auth/sessionCoordinator";
import { decodeJwtSub } from "../lib/auth/jwt";

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
  // App only wraps AppContent in LanguageProvider; AppContent owns the guards.
  const app = source.statements.find(
    (node): node is ts.FunctionDeclaration =>
      ts.isFunctionDeclaration(node) && node.name?.text === "AppContent",
  );
  assert.ok(app?.body);
  const names = new Set([
    "navigationGuardsRef",
    "criticalNavigationGuards",
    "navigationGuardRevision",
    "onNavigationGuardChange",
    "isNavigationBlocked",
  ]);
  const guardDeclarations = app.body.statements.filter(
    (node) =>
      ts.isVariableStatement(node) &&
      node.declarationList.declarations.some((declaration) =>
        ts.isIdentifier(declaration.name)
          ? names.has(declaration.name.text)
          : ts.isArrayBindingPattern(declaration.name) &&
            declaration.name.elements.some(
              (element) =>
                ts.isBindingElement(element) &&
                ts.isIdentifier(element.name) &&
                names.has(element.name.text),
            ),
      ),
  );
  assert.equal(guardDeclarations.length, names.size);
  const guardSource = `function AppGuardState() {
    ${guardDeclarations.map((node) => node.getText(source)).join("\n")}
    return { onNavigationGuardChange, isNavigationBlocked };
  }
exports.AppGuardState = AppGuardState;`;
  return (
    ["getActiveRouteName", "ThemedAppShell"]
      .map((name) => {
        const declaration = source.statements.find(
          (node) => ts.isFunctionDeclaration(node) && node.name?.text === name,
        );
        assert.ok(declaration, `Missing actual App declaration ${name}`);
        return declaration.getText(source);
      })
      .join("\n") +
    "\nexports.ThemedAppShell = ThemedAppShell;\n" +
    guardSource
  );
}

function setup(
  deferFade = false,
  initialWidth = 390,
  initiallyAuthenticated = true,
  cornerWindow = false,
) {
  const fades: (() => void)[] = [];
  const sequence: string[] = [];
  let keyboardDismissals = 0;
  let timerId = 0;
  const timers = new Map<number, { callback: () => void; delay: number }>();
  let rootState = fixture();
  let authenticated = initiallyAuthenticated;
  let accountId: string | null = initiallyAuthenticated ? "account-a" : null;
  const accountListeners = new Set<() => void>();
  let width = initialWidth;
  const testGuard = Symbol("critical-write");
  let appGuards!: {
    onNavigationGuardChange: (
      id: symbol,
      blocked: boolean,
      critical?: boolean,
    ) => void;
    isNavigationBlocked: () => boolean;
  };
  let authEpoch = 0;
  let ready = true;
  let rootReads = 0;
  let routeName: string | undefined = "Assistant";
  const setCurrentRouteName = (name: string | undefined) => {
    routeName = name;
  };
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
    useMemo: <T>(factory: () => T, dependencies: unknown[]) => {
      const i = cursor++;
      const previous = slots[i] as { value: T; dependencies: unknown[] } | undefined;
      if (!previous || !same(previous.dependencies, dependencies))
        slots[i] = { value: factory(), dependencies };
      return (slots[i] as { value: T }).value;
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
  const exports: {
    ThemedAppShell?: (props: object) => Element;
    AppGuardState?: () => typeof appGuards;
  } = {};
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
    useSafeAreaInsets: () => ({ top: 40, right: cornerWindow ? 83 : 0 }),
    Platform: {OS:"ios"},
    workspaceCornerSensor,
    useDiagnosticsNotice: () => {},
    useLaunchShell: () => {},
    useAuthSession: () => ({ isAuthenticated: authenticated }),
    getAuthAccountId: () => accountId,
    getAuthEpoch: () => authEpoch,
    subscribeAuthAccount: (listener: () => void) => {
      accountListeners.add(listener);
      return () => accountListeners.delete(listener);
    },
    useWindowDimensions: () => ({ width, height: 1024 }),
    useAppLanguage: () => ({ language: "en", t: translateApp }),
    translateApp,
    newAssistantChatParams,
    workspaceSidebarLayout,
    findNodeHandle: () => 300,
    Keyboard: { dismiss: () => { keyboardDismissals++; } },
    ...Object.fromEntries(
      [
        "NavigationContainer",
        "StatusBar",
        "AppNavigator",
        "View",
        "ToastOverlay",
        "WorkspaceDrawer",
          "WorkspacePushShell",
        "WorkspaceQuickRail",
      ].map((name) => [name, name]),
    ),
    WorkspaceAccessibilityContext: {
      Provider: "WorkspaceAccessibilityProvider",
    },
    WorkspaceSidebarToggleContext: {
      Provider: "WorkspaceSidebarToggleProvider",
    },
    // The saved sidebar choice: restored and closed, as on a fresh device.
    useSidebarOpen: () => {
      const [open, setOpen] = react.useState(false);
      return [open, setOpen, true] as const;
    },
    workspaceAccessibilityState,
    workspaceFocus: focus,
    appLinking: {},
    pushToastStyles: { overlay: {} },
    appRootStyles: { fill: { flex: 1 } },
    workspaceNavigateAction,
    workspaceChatNavigationAction,
    openScannerSheetAction,
    openWorkspaceSheetAction,
    isWorkspaceSidebarRoot,
    activeWorkspaceChatTarget,
    workspaceDestinationMatches,
    workspaceContentIdentity,
  });
  const component = exports.ThemedAppShell;
  assert.ok(component);
  const currentRoute = (
    state: NavigationState | PartialState<NavigationState> = rootState,
  ): { name: string; params?: object } => {
    const route = state.routes[state.index ?? 0];
    assert.ok(route, "the navigator has an active route");
    return route.state ? currentRoute(route.state) : route;
  };
  const navigationRef = {
    isReady: () => ready,
    getRootState: () => {
      rootReads++;
      return rootState;
    },
    getCurrentRoute: () => currentRoute(),
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
        appGuards = exports.AppGuardState!();
        rendered = component!({
          navigationRef,
          currentRouteName: routeName,
          setCurrentRouteName,
          flushPendingNotificationNavigation: () => {},
          onNavigationGuardChange: appGuards.onNavigationGuardChange,
          navigationBlocked: appGuards.isNavigationBlocked(),
          isNavigationBlocked: appGuards.isNavigationBlocked,
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
    keyboardDismissals: () => keyboardDismissals,
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
    find,
    shell: () => find("WorkspacePushShell"),
    accessibilityValue: () => find("WorkspaceAccessibilityProvider").props.value,
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
    routeName: () => routeName,
    rootReads: () => rootReads,
    changeAccount: (next: string | null, batchSignOut = false) => {
      if (batchSignOut) {
        accountId = null;
        for (const listener of accountListeners) listener();
        authEpoch++;
      }
      accountId = next;
      authEpoch++;
      for (const listener of accountListeners) listener();
      dirty = true;
      flush();
    },
    authenticate: (
      next: boolean,
      trackBeforeEffects = false,
      commitScreenEffects?: () => void,
    ) => {
      if (authenticated !== next) authEpoch++;
      authenticated = next;
      accountId = next ? "account-a" : null;
      if (!next) focus.setModalActive(nativeMenuOwner, false);
      dirty = true;
      if (trackBeforeEffects || commitScreenEffects) {
        dirty = false;
        cursor = 0;
        appGuards = exports.AppGuardState!();
        rendered = component!({
          navigationRef,
          currentRouteName: routeName,
          setCurrentRouteName,
          flushPendingNotificationNavigation: () => {},
          onNavigationGuardChange: appGuards.onNavigationGuardChange,
          navigationBlocked: appGuards.isNavigationBlocked(),
          isNavigationBlocked: appGuards.isNavigationBlocked,
          pushToast: null,
        });
        if (trackBeforeEffects)
          find("NavigationContainer").props.onStateChange();
        commitScreenEffects?.();
      }
      flush();
    },
    readiness: (next: boolean) => {
      ready = next;
    },
    resize: (next: number) => {
      width = next;
      dirty = true;
      flush();
    },
    blockNavigation: (blocked: boolean) => {
      appGuards.onNavigationGuardChange(testGuard, blocked, true);
      flush();
    },
    guardWithoutRender: (blocked: boolean) =>
      appGuards.onNavigationGuardChange(testGuard, blocked, true),
    liveGetter: () => appGuards.isNavigationBlocked,
    hasDrawer: () =>
      elements(rendered).some((item) => item.type === "WorkspaceDrawer"),
  };
}

/**
 * Run the actual login and push activation effects, holding only native/API I/O.
 * This verifies JS navigation liveness, not OS modal dismissal or native touch.
 */
function postLoginSession(env: ReturnType<typeof setup>) {
  type AuthValue = ReturnType<
    typeof import("../features/auth/AuthSessionContext").useAuthSession
  >;
  type Provider =
    typeof import("../features/auth/AuthSessionContext").AuthSessionProvider;
  const permission = deferred<{ status: "granted" | "denied" }>();
  const registration = deferred<void>();
  const refresh = deferred<boolean>();
  const stored = new Map<string, string>();
  const storage = {
    getItemAsync: async (key: string) => stored.get(key) ?? null,
    setItemAsync: async (key: string, value: string) => {
      stored.set(key, value);
    },
    deleteItemAsync: async (key: string) => {
      stored.delete(key);
    },
  };
  const session = createSessionCoordinator(storage, () => {});
  let accountId: string | null = null;
  let permissionRequests = 0;
  let registrationRequests = 0;
  let refreshRequests = 0;
  const h = hookHarness<AuthValue>(
    {
      "react-native": {
        Platform: { OS: "ios" },
        AppState: {
          currentState: "active",
          addEventListener: () => ({ remove() {} }),
        },
      },
      "@tanstack/react-query": {
        useQueryClient: () => ({
          cancelQueries: async () => {},
          clear() {},
        }),
      },
      "../../lib/crashReporting/runtime": { crashLifecycle: { disarm() {}, setRestoring() {}, refresh() {} } },
      "../../lib/auth/authEpoch": { advanceAuthEpoch() {} },
      "../../lib/auth/authAccount": {
        getAuthAccountId: () => accountId,
        setAuthAccountId: (next: string | null) => {
          accountId = next;
        },
      },
      "../../lib/auth/sessionStore": {
        loadStoredAuthSession: session.load,
        persistAuthSession: session.persist,
        clearStoredAuthSession: session.clear,
        authSessionGeneration: session.generation,
      },
      "../../lib/api/http": {
        setSessionInvalidationListener() {},
        refreshAccessTokenIfNeeded: () => {
          refreshRequests++;
          return refresh.promise;
        },
      },
      "../../lib/env": { ALLOWED_EMAILS: [] },
      "../../lib/telemetry": {
        identify() {},
        shutdownTelemetry() {},
      },
      "../../lib/auth/jwt": { decodeJwtSub },
      "../../lib/consent": {
        useMobileConsent: () => ({
          enabled: false,
          isHydrated: true,
          accountId: null,
        }),
      },
      "../../lib/notifications/approvalRefreshSignal": {},
      "./notificationDestination": {},
      "../env": {},
      "expo-notifications": {
        getPermissionsAsync: async () => ({ status: "undetermined" }),
        requestPermissionsAsync: () => {
          permissionRequests++;
          return permission.promise;
        },
        getDevicePushTokenAsync: async () => ({
          type: "ios",
          data: "test-device-push-token",
        }),
      },
      "expo-secure-store": storage,
      "expo-task-manager": {},
      "../api/mobileApi": {
        mobileApi: {
          registerPushToken: () => {
            registrationRequests++;
            return registration.promise;
          },
        },
      },
      "./approvalRefreshSignal": {},
      "../auth/authAccount": { getAuthAccountId: () => accountId },
    },
    { __DEV__: false },
  );
  h.mock(
    "../../lib/notifications/pushNotifications",
    h.load(
      fileURLToPath(
        new URL("../lib/notifications/pushNotifications.ts", import.meta.url),
      ),
    ),
  );
  const { AuthSessionProvider } = h.load(
    fileURLToPath(
      new URL("../features/auth/AuthSessionContext.tsx", import.meta.url),
    ),
  ) as { AuthSessionProvider: Provider };
  h.mount(() => {
    const value = AuthSessionProvider({ children: null }).props
      .value as AuthValue;
    env.authenticate(value.isAuthenticated);
    return value;
  });
  return {
    h,
    permission,
    registration,
    refresh,
    counts: () => ({
      permissionRequests,
      registrationRequests,
      refreshRequests,
    }),
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

test("with the sidebar closed, rail destinations and a sheet closing onto a thread keep later selections and the menu usable", () => {
  const env = setup();
  env.readyNavigation();
  const rail = () => env.shell().props.quickRail(() => {}) as Element;
  // Repeated rail destinations each dispatch directly; nothing waits for a
  // sidebar that was never opened.
  rail().props.onOpen({ kind: "destination", id: "notifications" });
  env.commitLast();
  assert.equal(env.sequence.includes("fade out"), false, "no slab swap");
  rail().props.onOpen({ kind: "destination", id: "new_chat" });
  env.commitLast();
  assert.equal(env.dispatched.length, 2);
  assert.equal(env.shell().props.visible, false);
  // Recent chats closes onto another thread (the sheet's own reset).
  rail().props.onOpen({ kind: "destination", id: "recent_chats" });
  env.commitLast();
  env.setState(
    apply(
      env.state(),
      workspaceChatNavigationAction(env.state(), {
        kind: "agent",
        agentId: "researcher",
        conversationId: "chat-recent",
      }),
    ),
  );
  env.changedNavigation();
  env.paint();
  rail().props.onOpen({ kind: "destination", id: "notifications" });
  env.flush();
  assert.equal(env.dispatched.length, 4, "selections are not left fenced");
  env.open();
  assert.equal(env.shell().props.visible, true, "the menu still opens");
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

test("the folded Duo quick rail reuses the sidebar's chat, Notifications and sheet actions", () => {
  const rail = (
    env: ReturnType<typeof setup>,
    onToggle = () => {},
    fullScreen = false,
  ) => env.shell().props.quickRail(fullScreen, onToggle) as Element;
  {
    const env = setup();
    env.readyNavigation();
    let toggles = 0;
    for (const fullScreen of [false, true]) {
      const element = rail(env, () => toggles++, fullScreen);
      assert.equal(element.type, "WorkspaceQuickRail");
      assert.equal(element.props.fullScreen, fullScreen);
      element.props.onToggleFullScreen();
    }
    assert.equal(toggles, 2, "the shell owns full screen in both directions");
  }
  for (const [id, page, params] of [
    ["scan_qr", "DeviceLogin", { start_scanner: true }],
    ["recent_chats", "RecentChats", undefined],
    ["terminal_login_code", "AgentKeyLogin", { mint: true }],
  ] as const) {
    const env = setup();
    env.readyNavigation();
    const before = content(env.state());
    rail(env).props.onOpen({ kind: "destination", id });
    env.commitLast();
    assert.strictEqual(content(env.state()), before, "the page stays beneath");
    const sheet = env
      .state()
      .routes.find((r) => r.name === "WorkspaceSheet")!.state!;
    const shown = sheet.routes[sheet.index ?? sheet.routes.length - 1]!;
    assert.equal(shown.name, page);
    assert.deepEqual(structuredClone(shown.params), params);
  }
  {
    const env = setup();
    env.readyNavigation();
    const before = env.state();
    rail(env).props.onOpen({ kind: "destination", id: "notifications" });
    env.flush();
    assert.deepEqual(
      comparable(env.dispatched),
      comparable([workspaceNavigateAction("Notifications", undefined, before)]),
    );
  }
  // Every side-menu destination quick access can hold opens the side menu's
  // own action for it.
  for (const [id, expected] of [
    ["automations", (state: any) => workspaceNavigateAction("Automations", undefined, state)],
    ["machines", (state: any) => workspaceNavigateAction("Machines", undefined, state)],
    ["plugins", (state: any) => workspaceNavigateAction("Plugins", undefined, state)],
    ["activity", (state: any) => workspaceNavigateAction("Activity", undefined, state)],
    ["create_agent", (state: any) => workspaceNavigateAction("AgentCreate", undefined, state)],
    ["agents", (state: any) => workspaceNavigateAction("Agents", { tab: "agents" }, state)],
    ["crews", (state: any) => workspaceNavigateAction("Agents", { tab: "groups" }, state)],
  ] as const) {
    const env = setup();
    env.readyNavigation();
    const before = env.state();
    rail(env).props.onOpen({ kind: "destination", id });
    env.flush();
    assert.deepEqual(comparable(env.dispatched), comparable([expected(before)]), id);
  }
  // A pinned agent or crew opens its chat, as its side-menu row does.
  for (const target of [
    { kind: "agent", agentId: "6f9619ff-8b86-d011-b42d-00c04fc964ff" },
    { kind: "group", groupId: "nyxg-0123456789abcdef0123456789abcdef" },
  ] as const) {
    const env = setup();
    env.readyNavigation();
    const before = env.state();
    rail(env).props.onOpen(
      target.kind === "agent"
        ? { kind: "agent", id: target.agentId }
        : { kind: "crew", id: target.groupId },
    );
    env.flush();
    // A fresh draft's id is time-based: compare everything else.
    const withoutDraft = (value: unknown) =>
      JSON.parse(
        JSON.stringify(comparable(value), (key, entry) =>
          key === "draftId" ? undefined : entry,
        ),
      );
    assert.deepEqual(
      withoutDraft(env.dispatched),
      withoutDraft([workspaceChatNavigationAction(before, target)]),
      target.kind,
    );
  }
  {
    // Settings: the sidebar gear's destination.
    const env = setup();
    env.readyNavigation();
    const before = env.state();
    rail(env).props.onOpen({ kind: "destination", id: "settings" });
    env.flush();
    assert.deepEqual(
      comparable(env.dispatched),
      comparable([workspaceNavigateAction("AccountSettings", undefined, before)]),
    );
  }
  {
    const env = setup();
    env.readyNavigation();
    rail(env).props.onOpen({ kind: "destination", id: "new_chat" });
    env.flush();
    const reset = env.dispatched.at(-1) as any;
    const workspace = reset.payload.routes.find(
      (route: any) => route.name === "Workspace",
    );
    const chat = workspace.state.routes.at(-1);
    assert.equal(chat.name, "Assistant");
    assert.equal(chat.params.draft, true, "a fresh, empty compose");
    assert.equal(chat.params.conversationId, undefined);
  }
  {
    const env = setup();
    env.readyNavigation();
    env.blockNavigation(true);
    rail(env).props.onOpen({ kind: "destination", id: "scan_qr" });
    rail(env).props.onOpen({ kind: "destination", id: "recent_chats" });
    assert.equal(env.dispatched.length, 0, "critical writes block rail sheets");
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

test("wide windows show the sidebar immediately and allow consecutive guarded destination selections", () => {
  const x = setup(false, 1194);
  assert.equal(x.drawer().props.visible, true);
  assert.equal(x.shell().props.presented, false);
  assert.equal(x.navigator().props.suppressWorkspaceAnimation, true);
  x.drawer().props.onNavigate("Machines");
  x.flush();
  assert.equal(x.dispatched.length, 1);
  assert.equal(x.watchdogs().length, 0);
  assert.equal(x.sequence.includes("fade out"), false);
  x.commitLast();
  x.drawer().props.onNavigate("Plugins");
  x.flush();
  assert.equal(x.dispatched.length, 2);
  assert.equal(x.drawer().props.visible, true);
  x.blockNavigation(true);
  x.drawer().props.onNavigate("Machines");
  assert.equal(
    x.dispatched.length,
    2,
    "a critical write still blocks sidebar navigation",
  );
  x.blockNavigation(false);
  x.drawer().props.onNavigate("Activity");
  assert.equal(x.dispatched.length, 3);
});

test("iPad resizing fences an unfinished compact selection and returns to a closed drawer", () => {
  const x = setup(true, 820);
  x.open();
  x.drawer().props.onNavigate("Machines");
  x.flush();
  assert.equal(x.watchdogs().length, 1);
  x.resize(1194);
  assert.equal(x.drawer().props.visible, true);
  assert.equal(x.shell().props.visible, false);
  assert.equal(x.shell().props.presented, false);
  assert.equal(x.watchdogs().length, 0);
  x.finishFade();
  x.flush();
  assert.equal(
    x.dispatched.length,
    0,
    "the old fade cannot change the resized navigator",
  );
  x.drawer().props.onNavigate("Plugins");
  x.commitLast();
  x.resize(507);
  assert.equal(x.drawer().props.visible, false);
  assert.equal(x.shell().props.presented, false);
  assert.equal(x.navigator().props.suppressWorkspaceAnimation, false);
  x.open();
  assert.equal(x.shell().props.visible, true);
});

test("persistent sidebar opens account sheets without hiding or resetting the selected chat", () => {
  const x = setup(false, 1024);
  x.drawer().props.onNavigate("AccountSettings");
  x.commitLast();
  assert.equal(x.shell().props.obscured, true);
  assert.equal(x.drawer().props.visible, true);
  assert.equal(x.shell().props.presented, false);
  x.authenticate(false);
  assert.equal(x.hasDrawer(), false);
});

test("sidebar menu and edge reveal dismiss the composer keyboard without navigating", () => {
  const menu = setup();
  menu.open();
  assert.equal(menu.keyboardDismissals(), 1);
  assert.equal(menu.shell().props.visible, true);
  assert.equal(menu.dispatched.length, 0);
  const edge = setup();
  edge.shell().props.onRevealStart();
  edge.flush();
  assert.equal(edge.keyboardDismissals(), 1);
  assert.equal(edge.dispatched.length, 0);
  const blocked = setup();
  blocked.blockNavigation(true);
  blocked.open();
  assert.equal(blocked.keyboardDismissals(), 0, "blocked navigation keeps the current editor focus");
  const wide = setup(false, 1194, true, true);
  wide.navigator().props.onOpenMenu();
  wide.flush();
  assert.equal(wide.keyboardDismissals(), 1);
  wide.navigator().props.onOpenMenu();
  wide.flush();
  assert.equal(wide.keyboardDismissals(), 1, "hiding a sidebar does not change input focus");
});

test("deferred persistent menu focus cannot land on a compact or sheet-covered sidebar", () => {
  // The wide sidebar starts closed; Menu opens it and moves focus there.
  const resized = setup(false, 1194, true, true);
  resized.navigator().props.onOpenMenu();
  resized.flush();
  resized.resize(507);
  resized.flushFocus();
  assert.deepEqual(resized.focused, []);

  const covered = setup(false, 1194, true, true);
  covered.navigator().props.onOpenMenu();
  covered.flush();
  covered.drawer().props.onNavigate("AccountSettings");
  covered.commitLast();
  covered.flushFocus();
  assert.deepEqual(covered.focused, []);

  const active = setup(false, 1194, true, true);
  active.navigator().props.onOpenMenu();
  active.flush();
  active.flushFocus();
  assert.deepEqual(active.focused, [300]);
  active.navigator().props.onOpenMenu();
  active.flush();
  active.flushFocus();
  assert.deepEqual(active.focused, [300], "hiding never moves focus");
});

test("on Duo wide windows the sidebar starts closed; Menu opens and hides it, labelled for VoiceOver; narrow windows keep the drawer", () => {
  const wide = setup(false, 1194, true, true);
  const shell = () => wide.find("WorkspacePushShell");
  const toggle = () => wide.find("WorkspaceSidebarToggleProvider");
  assert.equal(shell().props.sidebarCollapsed, true, "closed by default");
  assert.equal(shell().props.sidebarRestored, true);
  assert.deepEqual({ ...toggle().props.value }, { persistent: true, collapsed: true });
  wide.navigator().props.onOpenMenu();
  wide.flush();
  assert.equal(shell().props.sidebarCollapsed, false);
  assert.deepEqual({ ...toggle().props.value }, { persistent: true, collapsed: false });
  assert.equal(shell().props.visible, false, "no drawer opens on a wide window");
  wide.navigator().props.onOpenMenu();
  wide.flush();
  assert.equal(shell().props.sidebarCollapsed, true);
  // Critical writes block the toggle like other sidebar actions.
  wide.blockNavigation(true);
  wide.navigator().props.onOpenMenu();
  wide.flush();
  assert.equal(shell().props.sidebarCollapsed, true);

  const narrow = setup(false, 390);
  assert.equal(
    narrow.find("WorkspaceSidebarToggleProvider").props.value.persistent,
    false,
  );
  narrow.navigator().props.onOpenMenu();
  narrow.flush();
  assert.equal(narrow.find("WorkspacePushShell").props.visible, true, "the drawer opens");
});

test("critical writes block persistent sidebar Settings and QR/terminal sheet shortcuts", () => {
  const x = setup(false, 1194);
  x.blockNavigation(true);
  x.drawer().props.onNavigate("AccountSettings");
  x.drawer().props.onQuickAction("scanner");
  x.drawer().props.onQuickAction("terminal");
  assert.equal(x.dispatched.length, 0);
  x.blockNavigation(false);
  x.drawer().props.onNavigate("AccountSettings");
  assert.equal(x.dispatched.length, 1);
});

for (const outcome of [
  "granted",
  "denied",
  "permission-error",
  "registration-error",
] as const) {
  test(`first login keeps drawer navigation usable during pending notification activation (${outcome})`, async (t) => {
    const env = setup(false, 390, false);
    const login = postLoginSession(env);
    t.after(() => {
      login.permission.resolve({ status: "denied" });
      login.registration.resolve();
      login.refresh.resolve(false);
      login.h.unmount();
    });
    await login.h.settle();
    assert.equal(login.h.value().isRestoring, false);
    assert.equal(env.hasDrawer(), false);
    let completed = false;
    const signingIn = login.h
      .value()
      .signInWithSession({
        accessToken: `test.${Buffer.from(
          JSON.stringify({ sub: "newly-signed-in-user" }),
        ).toString("base64url")}.test`,
      })
      .then(() => {
        completed = true;
      });
    await login.h.settle();
    assert.equal(login.h.value().isAuthenticated, true);
    assert.equal(completed, false, "notification activation remains pending");
    assert.deepEqual(login.counts(), {
      permissionRequests: 1,
      registrationRequests: 0,
      refreshRequests: 1,
    });
    env.readyNavigation();

    const useDrawer = (route: "Machines" | "Automations" | "Activity") => {
      const before = env.dispatched.length;
      env.open();
      assert.equal(env.shell().props.visible, true);
      assert.equal(env.shell().props.presented, true);
      env.drawer().props.onNavigate(route);
      assert.equal(env.dispatched.length, before + 1);
      env.commitLast();
      assert.equal(env.shell().props.visible, false);
      env.drawer().props.onDismiss();
      env.flush();
      assert.equal(env.shell().props.presented, false);
      env.open();
      assert.equal(env.shell().props.visible, true);
      // Same close callback used by the actual shell's exposed-page Pressable.
      env.shell().props.onOpenChange(false);
      env.flush();
      env.drawer().props.onDismiss();
      env.flush();
      assert.equal(env.shell().props.presented, false);
    };

    useDrawer("Machines");
    if (outcome === "permission-error") {
      login.permission.reject(new Error("native permission unavailable"));
    } else {
      login.permission.resolve({
        status: outcome === "denied" ? "denied" : "granted",
      });
    }
    await login.h.settle();
    if (outcome === "granted" || outcome === "registration-error") {
      assert.equal(login.counts().registrationRequests, 1);
      assert.equal(
        completed,
        false,
        "backend push registration remains pending",
      );
      useDrawer("Automations");
      if (outcome === "registration-error") {
        login.registration.reject(new Error("push registration offline"));
      } else login.registration.resolve();
    } else assert.equal(login.counts().registrationRequests, 0);
    await signingIn;
    await login.h.settle();
    assert.equal(completed, true);
    assert.equal(login.h.value().isAuthenticated, true);
    useDrawer("Activity");
    login.refresh.resolve(false);
    await login.h.settle();
    assert.equal(env.shell().props.visible, false);
    assert.equal(env.shell().props.presented, false);
  });
}


for (const width of [390, 900, 1194]) {
  test(`critical writes registered after opening disable the ${width}pt drawer and settle without reopening`, () => {
    const env = setup(false, width);
    if (width < 900) env.open();
    assert.equal(env.drawer().props.disabled, false);
    const getter = env.liveGetter();
    env
      .navigator()
      .props.onNavigationGuardChange(Symbol.for("late-write"), true, true);
    env.flush();
    assert.equal(env.drawer().props.disabled, true);
    assert.equal(env.drawer().props.visible, true);
    assert.equal(
      env.liveGetter(),
      getter,
      "getter stays stable across App renders",
    );
    env.drawer().props.onNavigate("Machines");
    env.drawer().props.onQuickAction("scanner");
    assert.equal(env.dispatched.length, 0);
    env
      .navigator()
      .props.onNavigationGuardChange(Symbol.for("late-write"), false);
    env.flush();
    assert.equal(env.drawer().props.disabled, false);
    assert.equal(env.drawer().props.visible, true);
    env.drawer().props.onNavigate("Machines");
    assert.equal(env.dispatched.length, 1);
  });
}

test("a critical write cannot trap a compact drawer: menu toggle and exposed-sliver close remain available", () => {
  for (const close of ["menu", "sliver"] as const) {
    const env = setup();
    env.open();
    env.blockNavigation(true);
    assert.equal(env.drawer().props.disabled, true);
    if (close === "menu") env.navigator().props.onOpenMenu();
    else env.shell().props.onOpenChange(false);
    env.flush();
    assert.equal(env.shell().props.visible, false, close);
    env.drawer().props.onDismiss();
    env.flush();
    assert.equal(env.shell().props.presented, false, close);
    assert.equal(env.dispatched.length, 0);
  }
});

for (const action of [
  "destination",
  "chat",
  "quick action",
  "settings",
  "search",
  "create agent",
  "create crew",
] as const) {
  test(`drawer ${action} reads the live guard when it changes between render and tap`, () => {
    const env = setup();
    env.open();
    const props = env.drawer().props;
    const invoke = (props: Element["props"]) => {
      if (action === "destination") props.onNavigate("Machines");
      else if (action === "chat") props.onOpenChat({ kind: "home" });
      else if (action === "quick action") props.onQuickAction("terminal");
      else if (action === "settings") props.onNavigate("AccountSettings");
      else if (action === "search") props.onBrowse("WorkspaceSearch");
      else if (action === "create agent") props.onCreateAgent();
      else props.onCreateGroup();
    };
    env.guardWithoutRender(true);
    invoke(props);
    assert.equal(env.dispatched.length, 0);
    env.flush();
    assert.equal(env.drawer().props.disabled, true);
    const blockedProps = env.drawer().props;
    env.guardWithoutRender(false);
    invoke(blockedProps);
    assert.equal(
      env.dispatched.length,
      1,
      "release takes effect before the next render",
    );
    env.flush();
    assert.equal(env.drawer().props.disabled, false);
  });
}

test("a guard registered during fade cancels the delayed dispatch and restores visible content", () => {
  const env = setup(true);
  env.open();
  env.drawer().props.onNavigate("Machines");
  env.guardWithoutRender(true);
  env.finishFade();
  env.flush();
  assert.equal(env.dispatched.length, 0);
  assert.equal(env.watchdogs().length, 0);
  assert.equal(env.shell().props.visible, true);
  assert.equal(env.sequence.at(-1), "fade in");
  env.blockNavigation(false);
  env.drawer().props.onNavigate("Plugins");
  env.finishFade();
  assert.equal(env.dispatched.length, 1);
});

test("first sign-in Home loading keeps destinations and quick actions usable", async (t) => {
  const env = setup(false, 390, false);
  const login = postLoginSession(env);
  t.after(() => {
    login.permission.resolve({ status: "denied" });
    login.registration.resolve();
    login.refresh.resolve(false);
    login.h.unmount();
  });
  await login.h.settle();
  void login.h.value().signInWithSession({
    accessToken: `test.${Buffer.from(JSON.stringify({ sub: "first-login-home" })).toString("base64url")}.test`,
  });
  await login.h.settle();
  assert.equal(login.h.value().isAuthenticated, true);
  // Execute Home's actual disabled input alongside the actual App guard registry.
  // The screen harness separately covers this input through a full Home render.
  const filename = fileURLToPath(
    new URL("../features/assistant/AssistantScreen.tsx", import.meta.url),
  );
  const source = ts.createSourceFile(
    filename,
    readFileSync(filename, "utf8"),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let disabledInput: ts.Expression | undefined;
  const visit = (node: ts.Node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "useWorkspaceBack"
    ) {
      const input = node.arguments[0];
      if (input && ts.isObjectLiteralExpression(input)) {
        const property = input.properties.find(
          (item) =>
            ts.isPropertyAssignment(item) &&
            item.name.getText(source) === "disabled",
        );
        if (property && ts.isPropertyAssignment(property))
          disabledInput = property.initializer;
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(disabledInput);
  const disabled: unknown = runInNewContext(
    `(${disabledInput.getText(source)})`,
    {
      assistant: { loading: true, sending: false, conversationId: undefined },
      uploads: { session: { sending: false } },
    },
  );
  assert.equal(disabled, false);
  env
    .navigator()
    .props.onNavigationGuardChange(Symbol("Home"), disabled, disabled);
  env.flush();
  env.open();
  assert.equal(env.drawer().props.disabled, false);
  assert.equal(env.shell().props.visible, true);
  env.drawer().props.onQuickAction("scanner");
  env.drawer().props.onQuickAction("terminal");
  assert.equal(env.dispatched.length, 2);
  env.drawer().props.onNavigate("Machines");
  assert.equal(env.dispatched.length, 3);
  env.commitLast();
  env.drawer().props.onDismiss();
  env.flush();
  env.open();
  env.drawer().props.onNavigate("Automations");
  assert.equal(env.dispatched.length, 4);
});


test("menu opening and edge reveal check live critical guards before their next render", () => {
  for (const path of ["menu", "edge"] as const) {
    const env = setup();
    const reveal =
      path === "menu"
        ? env.navigator().props.onOpenMenu
        : env.shell().props.onRevealStart;
    env.guardWithoutRender(true);
    reveal();
    env.flush();
    assert.equal(env.shell().props.presented, false);
    const blockedReveal =
      path === "menu"
        ? env.navigator().props.onOpenMenu
        : env.shell().props.onRevealStart;
    env.guardWithoutRender(false);
    blockedReveal();
    env.flush();
    assert.equal(env.shell().props.presented, true);
    assert.equal(env.drawer().props.disabled, false);
  }
});

test("real workspace guard lifecycle releases re-render App and recover drawer actions", () => {
  type HeaderModule = {
    useWorkspaceBack: typeof import("./WorkspaceHeader").useWorkspaceBack;
    WorkspaceNavigationGuardContext: {
      value: import("./WorkspaceHeader").WorkspaceNavigationGuardChange;
    };
    WorkspaceSessionContext: { value: boolean };
  };
  for (const release of ["settle", "blur", "unmount", "signout"] as const) {
    const env = setup();
    env.open();
    let focused = true;
    const props = { active: false, disabled: true, onBack() {} };
    const page = hookHarness<void>({
      "@react-navigation/native": {
        useNavigation: () => ({ setOptions() {}, goBack() {}, dispatch() {} }),
        useIsFocused: () => focused,
        usePreventRemove() {},
      },
      "react-native": {
        StyleSheet: { create: (styles: unknown) => styles },
        BackHandler: { addEventListener: () => ({ remove() {} }) },
      },
      "lucide-react-native": {},
      "../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
      "../lib/accessibility/WorkspaceInteractionSurface": {},
      "../lib/accessibility/workspaceFocus": {},
    });
    const header = page.load(
      fileURLToPath(new URL("./WorkspaceHeader.tsx", import.meta.url)),
    ) as HeaderModule;
    header.WorkspaceNavigationGuardContext.value =
      env.navigator().props.onNavigationGuardChange;
    page.mount(() => header.useWorkspaceBack(props));
    env.flush();
    assert.equal(env.drawer().props.disabled, true);
    if (release === "settle") props.disabled = false;
    else if (release === "blur") focused = false;
    else if (release === "signout")
      header.WorkspaceSessionContext.value = false;
    if (release === "unmount") page.unmount();
    else page.render();
    env.flush();
    assert.equal(env.drawer().props.disabled, false, release);
    env.drawer().props.onNavigate("Machines");
    assert.equal(env.dispatched.length, 1, release);
    if (release !== "unmount") page.unmount();
  }
});


test("workspace accessibility context keeps its identity through unrelated shell renders", () => {
  const env = setup();
  const initial = env.accessibilityValue();
  env.blockNavigation(true);
  assert.equal(
    env.accessibilityValue(),
    initial,
    "guard updates do not invalidate every header",
  );
  env.navigator().props.onEdgeBackChange(Symbol("editor"), () => {});
  env.flush();
  assert.equal(env.accessibilityValue(), initial);
  env.blockNavigation(false);
  env.open();
  const open = env.accessibilityValue();
  assert.notEqual(open, initial);
  assert.equal(open.sidebarOpen, true);
  env.blockNavigation(true);
  assert.equal(env.accessibilityValue(), open);
  env.navigator().props.onOpenMenu();
  env.flush();
  env.drawer().props.onDismiss();
  env.flush();
  const closed = env.accessibilityValue();
  assert.notEqual(closed, open);
  assert.equal(closed.sidebarOpen, false);
});

for (const ordering of [
  "reset-before-tracking",
  "tracking-before-reset",
] as const) {
  test(`Settings sign-out then sign-in recovers menu and drawer destination with ${ordering}`, () => {
    const env = setup();
    env.readyNavigation();
    env.open();
    env.drawer().props.onNavigate("AccountSettings");
    env.commitLast();
    assert.equal(env.shell().props.obscured, true);
    const auth = hydrate({ routes: [{ name: "Auth" }] });
    if (ordering === "reset-before-tracking") {
      env.authenticate(false);
      env.setState(auth);
      env.changedNavigation();
    } else {
      env.setState(auth);
      env.authenticate(false, true);
    }
    env.authenticate(true);
    env.setState(
      hydrate({
        routes: [
          {
            name: "Workspace",
            state: {
              routes: [{ name: "Assistant", params: { home: true } }],
            },
          },
        ],
      }),
    );
    env.changedNavigation();
    env.open();
    assert.equal(
      env.shell().props.visible,
      true,
      "Open navigation must work in the new session",
    );
    const count = env.dispatched.length;
    env.drawer().props.onNavigate("Plugins");
    assert.equal(
      env.dispatched.length,
      count + 1,
      "a visible drawer destination must dispatch",
    );
    env.commitLast();
    assert.equal(content(env.state()).routes.at(-1)?.name, "Plugins");
    assert.equal(env.shell().props.visible, false);
  });
}

for (const transition of ["account-switch", "batched-same-account"] as const) {
  test(`${transition} resets the Settings drawer latch without an authentication render`, () => {
    const env = setup();
    env.readyNavigation();
    env.open();
    env.drawer().props.onNavigate("AccountSettings");
    env.commitLast();
    assert.equal(env.shell().props.obscured, true);
    const sheetContext = env.accessibilityValue();
    env.changeAccount(
      transition === "account-switch" ? "account-b" : "account-a",
      transition === "batched-same-account",
    );
    assert.equal(env.shell().props.visible, false);
    assert.equal(env.shell().props.presented, false);
    assert.equal(env.accessibilityValue().sheetOpen, true);
    assert.equal(env.accessibilityValue().primaryKey, sheetContext.primaryKey);
    env.setState(fixture());
    env.changedNavigation();
    env.open();
    assert.equal(env.shell().props.visible, true);
    const count = env.dispatched.length;
    env.drawer().props.onNavigate("Plugins");
    assert.equal(env.dispatched.length, count + 1);
    env.commitLast();
    assert.equal(content(env.state()).routes.at(-1)?.name, "Plugins");
  });
}

function mountWorkspaceBackScreen(
  env: ReturnType<typeof setup>,
  onBack: () => void,
) {
  type HeaderModule = {
    useWorkspaceBack: typeof import("./WorkspaceHeader").useWorkspaceBack;
    WorkspaceNavigationGuardContext: {
      value: import("./WorkspaceHeader").WorkspaceNavigationGuardChange;
    };
    WorkspaceEdgeBackContext: {
      value: import("./WorkspaceHeader").WorkspaceEdgeBackChange;
    };
  };
  const props = { active: true, disabled: true, onBack };
  const page = hookHarness<void>({
    "@react-navigation/native": {
      useNavigation: () => ({ setOptions() {}, goBack() {}, dispatch() {} }),
      useIsFocused: () => true,
      usePreventRemove() {},
    },
    "react-native": {
      StyleSheet: { create: (styles: unknown) => styles },
      BackHandler: { addEventListener: () => ({ remove() {} }) },
    },
    "lucide-react-native": {},
    "../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../lib/accessibility/WorkspaceInteractionSurface": {},
    "../lib/accessibility/workspaceFocus": {},
  });
  const header = page.load(
    fileURLToPath(new URL("./WorkspaceHeader.tsx", import.meta.url)),
  ) as HeaderModule;
  header.WorkspaceNavigationGuardContext.value =
    env.navigator().props.onNavigationGuardChange;
  header.WorkspaceEdgeBackContext.value =
    env.navigator().props.onEdgeBackChange;
  page.mount(() => header.useWorkspaceBack(props));
  return { props, render: page.render, unmount: page.unmount };
}

test("sign-in commit and account switches preserve mounted screen guards and edge-back callbacks", () => {
  const env = setup(false, 390, false);
  let backCalls = 0;
  let screen: ReturnType<typeof mountWorkspaceBackScreen> | undefined;
  env.authenticate(true, false, () => {
    screen = mountWorkspaceBackScreen(env, () => backCalls++);
    assert.equal(
      env.liveGetter()(),
      true,
      "child registers before the parent reset",
    );
  });
  assert.ok(screen);
  for (const boundary of ["sign-in", "account-switch"] as const) {
    if (boundary === "account-switch") env.changeAccount("account-b");
    assert.equal(env.liveGetter()(), true, boundary);
    assert.equal(env.drawer().props.disabled, true, boundary);
    assert.equal(typeof env.shell().props.onEdgeBack, "function", boundary);
    env.navigator().props.onOpenMenu();
    env.drawer().props.onNavigate("Plugins");
    env.flush();
    assert.equal(env.shell().props.presented, false, boundary);
    assert.equal(env.dispatched.length, 0, boundary);
  }
  screen.props.disabled = false;
  screen.render();
  env.flush();
  assert.equal(env.liveGetter()(), false);
  env.shell().props.onEdgeBack();
  assert.equal(
    backCalls,
    1,
    "the registered edge-back survives the resets and write settlement",
  );
  env.open();
  env.drawer().props.onNavigate("Plugins");
  assert.equal(env.dispatched.length, 1);
  screen.unmount();
  env.flush();
  assert.equal(env.shell().props.onEdgeBack, undefined);
});

test("unmounting the old session's screen releases its guard and edge-back before the next sign-in", () => {
  const env = setup();
  const screen = mountWorkspaceBackScreen(env, () => {});
  env.flush();
  assert.equal(env.liveGetter()(), true);
  assert.equal(typeof env.shell().props.onEdgeBack, "function");
  env.authenticate(false);
  screen.unmount();
  env.flush();
  assert.equal(env.liveGetter()(), false);
  assert.equal(env.shell().props.onEdgeBack, undefined);
  env.authenticate(true);
  assert.equal(env.drawer().props.disabled, false);
  env.open();
  assert.equal(env.shell().props.visible, true);
  env.drawer().props.onNavigate("Plugins");
  assert.equal(env.dispatched.length, 1);
});

test("account switch preserves a mounted sheet's accessibility and obscured focus context", () => {
  const env = setup();
  env.readyNavigation();
  env.open();
  env.drawer().props.onNavigate("AccountSettings");
  env.commitLast();
  const scope = env.accessibilityValue().primaryKey;
  env.changeAccount("account-b");
  assert.equal(env.accessibilityValue().sheetOpen, true);
  assert.equal(env.accessibilityValue().primaryKey, scope);
  assert.equal(env.shell().props.obscured, true);
  env.drawer().props.onDismiss();
  env.flush();
  env.focus.register(Symbol("new account header"), () => 77, true, scope);
  env.focus.restore(env.focus.capture(scope));
  env.flushFocus();
  assert.deepEqual(
    env.focused,
    [],
    "the live sheet still obscures its underlying header",
  );
  env.setState(apply(env.state(), dismissWorkspaceSheetAction(env.state())));
  env.changedNavigation();
  env.flushFocus();
  assert.equal(env.accessibilityValue().sheetOpen, false);
  assert.deepEqual(env.focused, [77]);
});

test("session reset uses neutral navigation context only before navigator readiness", () => {
  const env = setup();
  env.open();
  env.drawer().props.onNavigate("AccountSettings");
  env.commitLast();
  assert.equal(env.accessibilityValue().sheetOpen, true);
  env.readiness(false);
  env.changeAccount("account-b");
  assert.equal(env.accessibilityValue().sheetOpen, false);
  assert.equal(env.accessibilityValue().primaryKey, undefined);
  assert.equal(env.shell().props.obscured, false);
});

test("account changes cancel pending drawer dispatches and their selection watchdogs", () => {
  for (const phase of ["fade", "paint"] as const) {
    const env = setup(true);
    env.open();
    env.drawer().props.onNavigate("Plugins");
    if (phase === "paint") {
      env.finishFade();
      env.commitLast(false);
    }
    assert.equal(env.watchdogs().length, 1);
    const count = env.dispatched.length;
    env.changeAccount("account-b");
    assert.equal(env.watchdogs().length, 0);
    assert.equal(env.shell().props.visible, false);
    assert.equal(env.shell().props.presented, false);
    env.finishFade();
    env.expireWatchdog();
    assert.equal(env.dispatched.length, count, phase);
    env.open();
    env.drawer().props.onNavigate("Machines");
    env.finishFade();
    assert.equal(env.dispatched.length, count + 1, phase);
    env.commitLast();
    assert.equal(content(env.state()).routes.at(-1)?.name, "Machines");
  }
});

test("a prior session's paint callback cannot settle a new account's drawer selection", () => {
  const env = setup(true);
  const oldPaint = env.navigator().props.onWorkspacePagePainted;
  env.changeAccount("account-b");
  env.open();
  env.drawer().props.onNavigate("Plugins");
  env.finishFade();
  const next = apply(env.state(), env.dispatched.at(-1)!);
  oldPaint(workspaceContentIdentity(next));
  env.setState(next);
  env.changedNavigation();
  assert.equal(
    env.shell().props.visible,
    true,
    "only the current session may confirm paint",
  );
  env.paint();
  assert.equal(env.shell().props.visible, false);
});

test("sign-out account invalidation fences menu and quick actions before the auth UI changes", () => {
  const env = setup();
  env.readyNavigation();
  env.open();
  const quickAction = env.drawer().props.onQuickAction;
  env.changeAccount(null);
  assert.equal(
    env.hasDrawer(),
    true,
    "authentication UI has not committed yet",
  );
  assert.equal(env.shell().props.presented, false);
  env.open();
  env.shell().props.onRevealStart();
  quickAction("scanner");
  env.drawer().props.onNavigate("AccountSettings");
  env.flush();
  assert.equal(env.shell().props.presented, false);
  assert.equal(env.dispatched.length, 0);
  env.setState(hydrate({ routes: [{ name: "Auth" }] }));
  env.changedNavigation();
  assert.equal(env.routeName(), "Auth");
  assert.equal(env.accessibilityValue().sheetOpen, false);
  env.changeAccount("account-b");
  env.setState(fixture());
  env.changedNavigation();
  env.open();
  assert.equal(env.shell().props.visible, true);
  env.drawer().props.onQuickAction("scanner");
  assert.equal(env.dispatched.length, 1);
});

test("session boundaries clear selected chat metadata and stale drawer focus restoration", () => {
  const env = setup();
  env.setState(pushContent(env.state(), "GroupChat", { groupId: "private-a" }));
  env.readyNavigation();
  assert.equal(env.drawer().props.currentChat.groupId, "private-a");
  assert.equal(env.routeName(), "GroupChat");
  const scope = env.accessibilityValue().primaryKey;
  const oldOwner = Symbol("old account header");
  env.focus.register(oldOwner, () => 71, true, scope);
  env.focus.invoked(oldOwner);
  env.open();
  env.shell().props.onOpenChange(false);
  env.flush();
  env.drawer().props.onDismiss();
  env.flush();
  env.changeAccount("account-b");
  assert.equal(env.drawer().props.currentChat, undefined);
  assert.equal(env.accessibilityValue().primaryKey, scope);
  env.flushFocus();
  assert.deepEqual(env.focused, []);
  env.setState(
    hydrate({
      routes: [
        {
          name: "Workspace",
          state: {
            routes: [{ name: "Assistant", params: { home: true } }],
          },
        },
      ],
    }),
  );
  env.changedNavigation();
  assert.equal(env.routeName(), "Assistant");
  assert.equal(env.drawer().props.currentChat.kind, "home");
  env.focus.register(
    Symbol("new account header"),
    () => 72,
    true,
    env.accessibilityValue().primaryKey,
  );
  env.open();
  env.shell().props.onOpenChange(false);
  env.flush();
  env.drawer().props.onDismiss();
  env.flush();
  env.flushFocus();
  assert.deepEqual(env.focused, [72]);
});


test("P9 ordinary wide App retains a visible sidebar despite a stored Duo choice", () => {
  const wide = setup(false,1194);
  assert.equal(wide.find("WorkspacePushShell").props.sidebarCollapsed, false);
  assert.deepEqual({...wide.find("WorkspaceSidebarToggleProvider").props.value}, {persistent:false, collapsed:false});
});
