import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";
import type { NavigationAction } from "@react-navigation/native";
import type {
  useWorkspaceBack,
  workspaceHeaderOptions,
} from "./WorkspaceHeader";

type Element = { type: unknown; props: Record<string, any> };
type Context<T = unknown> = { value: T };
type Effect = {
  dependencies?: unknown[];
  cleanup?: () => void;
  run: () => void | (() => void);
};
type AlertButton = { text: string; onPress?: () => void };
type BeforeRemove = (event: {
  data: { action: NavigationAction };
  preventDefault: () => void;
}) => void;
type BackProps = Parameters<typeof useWorkspaceBack>[0];
const same = (a?: unknown[], b?: unknown[]) =>
  !!a && !!b && a.length === b.length && a.every((v, i) => Object.is(v, b[i]));

/** Execute the real header and React Navigation removal hook at their native boundaries. */
function setup({
  inSheet = true,
  ...initial
}: Partial<BackProps> & { inSheet?: boolean } = {}) {
  const slots: unknown[] = [];
  const effects = new Map<number, Effect>();
  const layoutEffects = new Map<number, Effect>();
  let pendingEffects: number[] = [];
  let pendingLayouts: number[] = [];
  let cursor = 0;
  let dirty = false;
  let focused = true;
  let localBacks = 0;
  let nativeBacks = 0;
  let menuOpens = 0;
  let closePresses = 0;
  let cancelledNavigations = 0;
  let header: ReturnType<typeof workspaceHeaderOptions>;
  let preventionId = 0;
  const listeners = new Set<BeforeRemove>();
  const hardware = new Set<() => boolean>();
  const guards = new Map<string, boolean>();
  const dismissGuards = new Map<symbol, () => Promise<boolean>>();
  const guardChanges: boolean[] = [];
  const criticalChanges: boolean[] = [];
  const edgeBacks = new Map<symbol, () => void>();
  const dispatched: NavigationAction[] = [];
  const dispatchPrevention: boolean[] = [];
  const alerts: { title: string; message: string; buttons: AlertButton[] }[] =
    [];
  const rootAction: NavigationAction = {
    type: "RESET",
    target: "root-stack",
    payload: {
      routes: [{ name: "Workspace", key: "underlying-chat" }],
      index: 0,
    },
  };
  const props: BackProps = {
    active: false,
    onBack: () => {
      localBacks++;
    },
    ...initial,
  };
  function effect(
    map: Map<number, Effect>,
    pending: number[],
    run: Effect["run"],
    dependencies?: unknown[],
  ) {
    const i = cursor++;
    const previous = map.get(i);
    if (!previous || !same(previous.dependencies, dependencies)) {
      map.set(i, { dependencies, run, cleanup: previous?.cleanup });
      pending.push(i);
    }
  }
  const react = {
    createContext: <T>(value: T): Context<T> => ({ value }),
    useContext: <T>(context: Context<T>) => context.value,
    useRef: <T>(value: T) => {
      const i = cursor++;
      slots[i] ??= { current: value };
      return slots[i] as { current: T };
    },
    useState: <T>(initialValue?: T | (() => T)) => {
      const i = cursor++;
      if (!(i in slots))
        slots[i] =
          typeof initialValue === "function"
            ? (initialValue as () => T)()
            : initialValue;
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
    useEffect: (run: Effect["run"], dependencies?: unknown[]) =>
      effect(effects, pendingEffects, run, dependencies),
    useLayoutEffect: (run: Effect["run"], dependencies?: unknown[]) =>
      effect(layoutEffects, pendingLayouts, run, dependencies),
  };
  const navigation = {
    setOptions: (options: ReturnType<typeof workspaceHeaderOptions>) => {
      header = options;
    },
    goBack: () => {
      nativeBacks++;
    },
    addListener: (_name: string, callback: BeforeRemove) => {
      listeners.add(callback);
      return () => listeners.delete(callback);
    },
    dispatch: (action: NavigationAction) => {
      let prevented = false;
      const event = {
        data: { action },
        preventDefault: () => {
          prevented = true;
        },
      };
      for (const listener of listeners) listener(event);
      dispatchPrevention.push([...guards.values()].some(Boolean));
      if (!prevented) dispatched.push(action);
      return prevented;
    },
  };
  const jsx = (type: unknown, elementProps: Record<string, unknown>) => ({
    type,
    props: elementProps,
  });
  const mocks: Record<string, unknown> = {
    react,
    "../lib/accessibility/WorkspaceInteractionSurface": {
      WorkspaceInteractionSurface: "WorkspaceInteractionSurface",
    },
    "../lib/accessibility/workspaceAccessibility": {
      useWorkspaceAccessibility: () => ({
        sheetOpen: inSheet,
        primaryKey: "chat",
      }),
    },
    "../lib/accessibility/workspaceFocus": {
      workspaceFocus: {
        release() {},
        invoked() {},
        focusDrawerOnce() {},
        register() {},
      },
    },
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Alert: {
        alert: (title: string, message: string, buttons: AlertButton[]) =>
          alerts.push({ title, message, buttons }),
      },
      BackHandler: {
        addEventListener: (_: string, callback: () => boolean) => {
          hardware.add(callback);
          return { remove: () => hardware.delete(callback) };
        },
      },
      Pressable: "Pressable",
      StyleSheet: { create: <T>(styles: T) => styles },
    },
    "lucide-react-native": { ChevronLeft: "ChevronLeft", Menu: "Menu", X: "X" },
    "../theme/ThemeContext": {
      useTheme: () => ({ colors: { textPrimary: "white" } }),
    },
    "nanoid/non-secure": { nanoid: () => `prevent-${++preventionId}` },
    "./useNavigation": { useNavigation: () => navigation },
    "./useRoute": { useRoute: () => ({ key: "editor-route" }) },
    "./usePreventRemoveContext": {
      usePreventRemoveContext: () => ({
        setPreventRemove: setPreventRemove,
      }),
    },
    "use-latest-callback": {
      default: <T>(callback: T) => {
        const current = react.useRef(callback);
        current.current = callback;
        return react.useCallback(
          (...args: unknown[]) =>
            (current.current as (...args: unknown[]) => unknown)(...args),
          [],
        );
      },
      __esModule: true,
    },
  };
  function setPreventRemove(id: string, _route: string, blocked: boolean) {
    guards.set(id, blocked);
  }
  function load(filename: string) {
    const require = createRequire(filename);
    const output = ts.transpileModule(readFileSync(filename, "utf8"), {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
        jsx: ts.JsxEmit.ReactJSX,
        esModuleInterop: true,
      },
    }).outputText;
    const exports: Record<string, unknown> = {};
    runInNewContext(
      output,
      {
        exports,
        require: (name: string) =>
          name in mocks ? mocks[name] : require(name),
      },
      { filename },
    );
    return exports;
  }
  const nativeFilename = fileURLToPath(
    new URL(
      "../../node_modules/@react-navigation/core/src/usePreventRemove.tsx",
      import.meta.url,
    ),
  );
  const nativeHook = load(nativeFilename).usePreventRemove;
  mocks["@react-navigation/native"] = {
    useNavigation: () => navigation,
    useIsFocused: () => focused,
    usePreventRemove: nativeHook,
  };
  const module = load(
    fileURLToPath(new URL("./WorkspaceHeader.tsx", import.meta.url)),
  ) as {
    useWorkspaceBack: typeof useWorkspaceBack;
    workspaceHeaderOptions: typeof workspaceHeaderOptions;
    WorkspaceSheetContext: Context;
    WorkspaceSessionContext: Context;
    WorkspaceMenuContext: Context;
    WorkspaceNavigationGuardContext: Context;
    WorkspaceEdgeBackContext: Context;
    WorkspaceNavigationCancelledContext: Context;
  };
  module.WorkspaceSheetContext.value = inSheet
    ? {
        registerDismissGuard: (id: symbol, guard: () => Promise<boolean>) => {
          dismissGuards.set(id, guard);
          return () => {
            dismissGuards.delete(id);
          };
        },
        rootKey: "root-stack",
        sheetKey: "workspace-sheet",
        onClose: () => {
          closePresses++;
          navigation.dispatch(rootAction);
        },
        onOpenChat: () => {},
        onOpenGroup: () => {},
      }
    : null;
  module.WorkspaceMenuContext.value = () => {
    menuOpens++;
  };
  module.WorkspaceNavigationGuardContext.value = (
    _id: symbol,
    blocked: boolean,
    critical = false,
  ) => {
    guardChanges.push(blocked);
    criticalChanges.push(critical);
  };
  module.WorkspaceNavigationCancelledContext.value = () =>
    cancelledNavigations++;
  module.WorkspaceEdgeBackContext.value = (id: symbol, back?: () => void) => {
    if (back) edgeBacks.set(id, back);
    else edgeBacks.delete(id);
  };

  function commit(map: Map<number, Effect>, pending: number[]) {
    for (const i of pending) map.get(i)?.cleanup?.();
    for (const i of pending) {
      const item = map.get(i)!;
      item.cleanup = item.run() || undefined;
    }
  }
  function flush() {
    let iterations = 0;
    while (dirty || pendingLayouts.length || pendingEffects.length) {
      if (++iterations > 30)
        throw new Error("Header removal guard did not settle");
      if (dirty) {
        dirty = false;
        cursor = 0;
        module.useWorkspaceBack(props);
      }
      const layouts = pendingLayouts;
      pendingLayouts = [];
      commit(layoutEffects, layouts);
      const passive = pendingEffects;
      pendingEffects = [];
      commit(effects, passive);
    }
  }
  function button(side: "left" | "right") {
    const renderer = side === "left" ? header.headerLeft : header.headerRight;
    const element = renderer?.({} as never) as unknown as Element | null;
    if (!element) return null;
    assert.equal(typeof element.type, "function");
    return (element.type as (props: Record<string, unknown>) => Element)(
      element.props,
    );
  }
  dirty = true;
  flush();
  return {
    preflight: async () => {
      for (const guard of dismissGuards.values())
        if (!(await guard())) return false;
      return true;
    },
    setSessionActive: (active: boolean) => {
      module.WorkspaceSessionContext.value = active;
      dirty = true;
      flush();
    },
    alerts,
    dispatched,
    dispatchPrevention,
    guardChanges,
    criticalChanges,
    edgeBack: () => [...edgeBacks.values()].at(-1)?.(),
    rootAction,
    cancelled: () => cancelledNavigations,
    button,
    flush,
    options: () => header,
    counts: () => ({ localBacks, nativeBacks, menuOpens, closePresses }),
    blocked: () => [...guards.values()].some(Boolean),
    attempt: navigation.dispatch,
    update: (next: Partial<BackProps>) => {
      Object.assign(props, next);
      dirty = true;
      flush();
    },
    focus: (next: boolean) => {
      focused = next;
      dirty = true;
      flush();
    },
    hardwareBack: () => [...hardware].reverse().some((callback) => callback()),
    choose: (text: string, settle = true) => {
      const item = alerts
        .at(-1)
        ?.buttons.find((candidate) => candidate.text === text);
      assert.ok(item, `Missing alert choice ${text}`);
      item.onPress?.();
      if (settle) flush();
    },
    unmount: () => {
      for (const map of [layoutEffects, effects])
        for (const item of map.values()) item.cleanup?.();
    },
  };
}

test("a drawer root shows Close while nested local history adds Back independently", () => {
  const env = setup();
  assert.equal(env.button("left"), null);
  assert.equal(env.button("right")?.props.accessibilityLabel, "Close drawer");
  assert.equal(env.button("right")?.props.children.type, "X");
  assert.equal(env.options().unstable_headerLeftItems?.({} as never).length, 0);
  assert.equal(
    env.options().unstable_headerRightItems?.({} as never).length,
    1,
  );
  env.update({ active: true, label: "Previous step" });
  const back = env.button("left");
  assert.equal(back?.props.accessibilityLabel, "Previous step");
  assert.equal(back?.props.children.type, "ChevronLeft");
  back?.props.onPress();
  assert.equal(env.counts().localBacks, 1);
  assert.equal(env.counts().closePresses, 0);
  assert.equal(env.options().gestureEnabled, true);
  env.button("right")?.props.onPress();
  env.flush();
  assert.equal(env.counts().localBacks, 1);
  assert.equal(env.dispatched.at(-1), env.rootAction);
});

test("outer root-targeted and sheet-sourced removals replay the exact original action", () => {
  for (const identity of [
    { target: "root-stack" },
    { source: "workspace-sheet" },
  ]) {
    const env = setup({ active: true });
    const marker = Symbol("native visited route keys");
    const action = {
      type: "RESET",
      payload: { routes: [{ name: "GroupChat" }] },
      ...identity,
      [marker]: new Set(["editor-route"]),
    };
    assert.equal(env.attempt(action), true);
    assert.equal(env.dispatched.length, 0);
    env.flush();
    assert.equal(env.counts().localBacks, 0);
    assert.equal(env.dispatched.length, 1);
    assert.equal(env.dispatched[0], action);
    assert.equal(
      env.dispatchPrevention[1],
      false,
      "replay occurs after the real usePreventRemove effect disables removal protection",
    );
    assert.equal(
      env.blocked(),
      true,
      "a failed/no-op dispatch does not permanently disable the guard",
    );
  }
});

test("a child route removal invokes local Back instead of closing the drawer", () => {
  const env = setup({ active: true });
  assert.equal(
    env.attempt({
      type: "GO_BACK",
      target: "sheet-inner-stack",
      source: "editor-route",
    }),
    true,
  );
  env.flush();
  assert.equal(env.counts().localBacks, 1);
  assert.deepEqual(env.dispatched, []);
  assert.equal(env.hardwareBack(), true);
  assert.equal(env.counts().localBacks, 2);
});

test("a native nested drawer route keeps Back and Close as separate destinations", () => {
  const env = setup({ active: false, defaultRole: "back" });
  assert.equal(env.button("left")?.props.accessibilityLabel, "Back");
  assert.equal(env.button("right")?.props.accessibilityLabel, "Close drawer");
  env.button("left")?.props.onPress();
  assert.equal(env.counts().nativeBacks, 1);
  assert.equal(env.counts().closePresses, 0);
  env.button("right")?.props.onPress();
  assert.equal(env.counts().nativeBacks, 1);
  assert.equal(env.dispatched[0], env.rootAction);
});

test("dirty drawer dismissal keeps editing or discards the entire drawer without local Back", () => {
  const env = setup({
    active: true,
    discardOnClose: { title: "Discard draft?", message: "Unsaved changes" },
  });
  env.attempt(env.rootAction);
  env.flush();
  assert.equal(env.alerts.at(-1)?.title, "Discard draft?");
  env.choose("Keep editing");
  assert.deepEqual(env.dispatched, []);
  assert.equal(env.blocked(), true);
  env.attempt(env.rootAction);
  env.choose("Discard");
  assert.equal(env.dispatched[0], env.rootAction);
  assert.equal(env.counts().localBacks, 0);
});

test("critical writes block local Back, hardware Back, and parent dismissal", () => {
  const env = setup({
    active: true,
    disabled: true,
    discardOnClose: { title: "Discard?", message: "Draft" },
  });
  assert.equal(env.button("left")?.props.disabled, true);
  assert.equal(env.button("right")?.props.disabled, true);
  assert.equal(env.options().gestureEnabled, false);
  env.attempt(env.rootAction);
  env.flush();
  env.attempt({ type: "GO_BACK", target: "sheet-inner-stack" });
  env.flush();
  assert.equal(env.hardwareBack(), true);
  assert.deepEqual(env.dispatched, []);
  assert.deepEqual(env.alerts, []);
  assert.equal(env.counts().localBacks, 0);
  assert.deepEqual(env.guardChanges, [true]);
  env.update({ disabled: false });
  assert.equal(
    env.guardChanges.at(-1),
    true,
    "dirty changes still defer external approval navigation",
  );
  env.update({ discardOnClose: undefined });
  assert.equal(env.guardChanges.at(-1), false);
});

test("dirty forms defer global notification navigation until saved or discarded", () => {
  const env = setup({
    active: false,
    discardOnClose: { title: "Discard?", message: "Draft" },
  });
  assert.deepEqual(env.guardChanges, [true]);
  env.update({ discardOnClose: undefined });
  assert.deepEqual(env.guardChanges, [true, false]);
  assert.equal(env.blocked(), false);
});

test("a critical write on the drawer root also blocks dismissal without local history", () => {
  const env = setup({ active: false, disabled: true });
  assert.equal(env.button("left"), null);
  assert.equal(env.button("right")?.props.disabled, true);
  assert.equal(env.options().gestureEnabled, false);
  assert.equal(env.attempt(env.rootAction), true);
  env.flush();
  assert.equal(env.hardwareBack(), true);
  assert.equal(env.counts().localBacks, 0);
  assert.deepEqual(env.dispatched, []);
});

test("a pending discard confirmation cannot dismiss after a critical write starts", () => {
  const env = setup({
    active: true,
    discardOnClose: { title: "Discard?", message: "Draft" },
  });
  env.attempt(env.rootAction);
  env.update({ disabled: true });
  env.choose("Discard");
  assert.deepEqual(env.dispatched, []);
  assert.equal(env.blocked(), true);
  assert.equal(env.counts().localBacks, 0);
});

test("critical writes starting in the confirmation batch cancel queued parent replay", () => {
  const env = setup({
    active: true,
    discardOnClose: { title: "Discard?", message: "Draft" },
  });
  env.attempt(env.rootAction);
  env.choose("Discard", false);
  env.update({ disabled: true });
  assert.deepEqual(env.dispatched, []);
  assert.equal(env.blocked(), true);
});

test("focus loss before confirmation commits cancels queued parent replay", () => {
  const env = setup({
    active: true,
    discardOnClose: { title: "Discard?", message: "Draft" },
  });
  env.attempt(env.rootAction);
  env.choose("Discard", false);
  env.focus(false);
  assert.deepEqual(env.dispatched, []);
  assert.equal(
    env.blocked(),
    true,
    "the hidden dirty editor still protects the entire drawer",
  );
});

test("a dirty editor beneath a shared legal or login entry still confirms parent drawer closure", () => {
  const env = setup({
    active: true,
    discardOnClose: {
      title: "Discard editor changes?",
      message: "Unsaved draft",
    },
  });
  env.focus(false);
  assert.equal(
    env.hardwareBack(),
    false,
    "the visible shared entry owns hardware Back",
  );
  assert.equal(env.attempt(env.rootAction), true);
  env.flush();
  assert.equal(env.alerts.at(-1)?.title, "Discard editor changes?");
  env.choose("Keep editing");
  assert.deepEqual(env.dispatched, []);
  assert.equal(env.blocked(), true);
  env.attempt(env.rootAction);
  env.choose("Discard");
  assert.equal(env.dispatched[0], env.rootAction);
  assert.equal(env.counts().localBacks, 0);
  assert.equal(env.dispatchPrevention.at(-1), false);
});

test("a hidden dirty editor continues blocking notification navigation until clean or unmounted", () => {
  const env = setup({
    active: true,
    discardOnClose: { title: "Discard?", message: "Draft" },
  });
  env.focus(false);
  assert.equal(env.guardChanges.at(-1), true);
  assert.equal(env.blocked(), true);
  env.update({ discardOnClose: undefined });
  assert.equal(env.guardChanges.at(-1), false);
  assert.equal(env.blocked(), false);
  env.update({ discardOnClose: { title: "Discard?", message: "New changes" } });
  assert.equal(env.guardChanges.at(-1), true);
  env.unmount();
  assert.equal(env.guardChanges.at(-1), false);
  assert.equal(env.blocked(), false);
});

test("dirty dismissal protection is independent of having a local detail view", () => {
  const env = setup({
    active: false,
    discardOnClose: { title: "Discard?", message: "Draft" },
  });
  assert.equal(env.button("left"), null);
  assert.equal(env.blocked(), true);
  env.button("right")?.props.onPress();
  assert.equal(env.alerts.length, 1);
  assert.equal(env.dispatched.length, 0);
  env.choose("Discard");
  assert.equal(env.dispatched[0], env.rootAction);
  assert.equal(env.counts().localBacks, 0);
});

test("pending discard callbacks lose authority after focus loss or unmount", () => {
  for (const boundary of ["blur", "unmount"]) {
    const env = setup({
      active: true,
      discardOnClose: { title: "Discard?", message: "Draft" },
    });
    env.attempt(env.rootAction);
    if (boundary === "blur") env.focus(false);
    else env.unmount();
    env.choose("Discard");
    assert.deepEqual(env.dispatched, []);
    assert.equal(env.counts().localBacks, 0);
  }
});

test("normal chat menu and contextual Back stay outside drawer dismissal behavior", () => {
  const main = setup({ inSheet: false });
  assert.equal(
    main.button("left")?.props.accessibilityLabel,
    "Open navigation",
  );
  assert.equal(main.button("left")?.props.children.type, "Menu");
  assert.equal(main.button("right"), null);
  assert.equal(
    main.options().unstable_headerRightItems?.({} as never).length,
    0,
  );
  main.button("left")?.props.onPress();
  assert.equal(main.counts().menuOpens, 1);
  assert.equal(main.blocked(), false);
  assert.equal(main.hardwareBack(), false);
  const contextual = setup({ inSheet: false, defaultRole: "back" });
  assert.equal(contextual.button("left")?.props.accessibilityLabel, "Back");
  assert.equal(contextual.button("right"), null);
  contextual.button("left")?.props.onPress();
  assert.equal(contextual.counts().nativeBacks, 1);
  contextual.update({ active: true });
  assert.equal(contextual.options().gestureEnabled, false);
  contextual.attempt({ type: "GO_BACK" });
  contextual.flush();
  assert.equal(contextual.counts().localBacks, 1);
  assert.equal(contextual.counts().menuOpens, 0);
});

test("losing focus and unmounting release critical-write and hardware guards", () => {
  const env = setup({ active: true, disabled: true });
  env.focus(false);
  assert.equal(env.blocked(), false);
  assert.equal(env.hardwareBack(), false);
  assert.deepEqual(env.guardChanges, [true, false]);
  env.focus(true);
  assert.equal(env.blocked(), true);
  env.unmount();
  assert.equal(env.blocked(), false);
  assert.equal(env.hardwareBack(), false);
  assert.deepEqual(env.guardChanges, [true, false, true, false]);
});

test("surface preflight awaits the existing dirty confirmation and rejects pending writes", async () => {
  const env = setup({
    discardOnClose: { title: "Discard draft?", message: "Unsaved changes" },
  });
  const cancelled = env.preflight();
  env.choose("Keep editing");
  assert.equal(await cancelled, false);
  const discarded = env.preflight();
  env.choose("Discard");
  assert.equal(await discarded, true);
  env.update({ disabled: true });
  assert.equal(await env.preflight(), false);
  assert.equal(env.counts().localBacks, 0);
  env.unmount();
});
test("a write starting while the discard alert is open blocks the pending dismissal", async () => {
  const env = setup({
    discardOnClose: { title: "Discard draft?", message: "Unsaved changes" },
  });
  const result = env.preflight();
  env.update({ disabled: true });
  env.choose("Discard");
  assert.equal(await result, false);
  env.unmount();
});
test("forced sign-out releases dirty and pending removal guards without a discard alert", async () => {
  const env = setup({
    disabled: true,
    discardOnClose: { title: "Discard draft?", message: "Unsaved changes" },
  });
  env.setSessionActive(false);
  assert.equal(env.blocked(), false);
  assert.equal(await env.preflight(), true);
  env.attempt(env.rootAction);
  env.flush();
  assert.equal(env.alerts.length, 0);
  assert.equal(env.dispatched.length, 1);
  env.unmount();
});

test("home RESET waits for dirty drawer confirmation and preserves the exact home target", () => {
  const env = setup({
    discardOnClose: { title: "Discard changes?", message: "Unsaved edits" },
  });
  const action: NavigationAction = {
    ...env.rootAction,
    payload: {
      index: 0,
      routes: [
        {
          name: "Workspace",
          state: {
            routes: [
              {
                name: "Assistant",
                params: { home: true, draft: false, openLatest: false },
              },
            ],
          },
        },
      ],
    },
  };
  assert.equal(env.attempt(action), true);
  assert.equal(env.alerts.at(-1)!.title, "Discard changes?");
  env.choose("Keep editing");
  assert.equal(env.dispatched.length, 0);
  assert.equal(env.attempt(action), true);
  env.choose("Discard");
  assert.equal(env.dispatched[0], action);
  env.unmount();
});

test("main-content sidebar and deep-link removals share dirty Cancel/Discard semantics", () => {
  for (const type of ["RESET", "NAVIGATE", "REPLACE", "POP_TO"]) {
    const env = setup({
      inSheet: false,
      active: true,
      discardOnClose: { title: "Discard changes?", message: "Unsaved editor" },
    });
    const action = { ...env.rootAction, type };
    assert.equal(env.attempt(action), true);
    assert.equal(env.alerts.at(-1)?.title, "Discard changes?");
    env.attempt(action);
    assert.equal(
      env.alerts.length,
      1,
      "one confirmation owns the pending leave intent",
    );
    env.choose("Keep editing");
    assert.equal(env.dispatched.length, 0);
    assert.equal(env.blocked(), true);
    env.attempt(action);
    env.choose("Discard");
    assert.equal(env.dispatched.at(-1), action);
    assert.equal(env.counts().localBacks, 0);
    env.unmount();
  }
});

test("dirty pages may reveal navigation but still defer external taps; critical writes block both", () => {
  const env = setup({
    inSheet: false,
    active: true,
    discardOnClose: { title: "Discard?", message: "Draft" },
  });
  assert.equal(env.guardChanges.at(-1), true);
  assert.equal(env.criticalChanges.at(-1), false);
  env.edgeBack();
  assert.equal(
    env.counts().localBacks,
    1,
    "edge Back uses the same editor callback as header and Android Back",
  );
  env.button("left")?.props.onPress();
  env.hardwareBack();
  assert.equal(env.counts().localBacks, 3);
  env.update({ disabled: true });
  assert.equal(env.criticalChanges.at(-1), true);
  env.edgeBack();
  assert.equal(env.counts().localBacks, 3);
  env.focus(false);
  env.edgeBack();
  assert.equal(env.counts().localBacks, 3);
  env.unmount();
});

test("main-content discard callbacks lose authority on sign-out, blur and unmount", () => {
  for (const boundary of ["signout", "blur", "unmount"]) {
    const env = setup({
      inSheet: false,
      active: true,
      discardOnClose: { title: "Discard?", message: "Draft" },
    });
    env.attempt(env.rootAction);
    if (boundary === "signout") env.setSessionActive(false);
    else if (boundary === "blur") env.focus(false);
    else env.unmount();
    env.choose("Discard");
    assert.equal(env.dispatched.length, 0, boundary);
  }
});

test("a dirty native leaf route confirms POP once and replays the exact action without a local Back loop", () => {
  for (const type of ["GO_BACK", "POP"]) {
    const env = setup({
      inSheet: false,
      active: false,
      defaultRole: "back",
      discardOnClose: { title: "Discard?", message: "Unsaved leaf" },
    });
    const action = { ...env.rootAction, type };
    env.attempt(action);
    assert.equal(env.alerts.length, 1);
    env.choose("Keep editing");
    assert.equal(env.dispatched.length, 0);
    env.attempt(action);
    env.choose("Discard");
    assert.equal(env.dispatched.at(-1), action);
    assert.equal(env.counts().localBacks, 0);
    env.unmount();
  }
});

test("the real dirty leave guard reports Keep editing to the sidebar but never cancels an accepted Discard", () => {
  const x = setup({
    inSheet: false,
    discardOnClose: { title: "Discard changes?", message: "Unsaved edits" },
  });
  x.attempt(x.rootAction);
  x.choose("Keep editing");
  assert.equal(x.cancelled(), 1);
  x.attempt(x.rootAction);
  x.choose("Discard");
  assert.equal(x.cancelled(), 1);
  assert.deepEqual(x.dispatched.at(-1), x.rootAction);
  x.unmount();
});
