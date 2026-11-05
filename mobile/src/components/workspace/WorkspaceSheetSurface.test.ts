import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { dismissWorkspaceSheetAction } from "../../app/navigationHierarchy";
import type {
  useWorkspaceBack,
  workspaceHeaderOptions,
} from "../../app/WorkspaceHeader";
import type { WorkspaceSheetHandle } from "./WorkspaceSheetSurface";

type Element = { type: unknown; props: Record<string, any> };
type Gesture = { dx: number; dy: number; vy: number };
type PanHandlers = {
  onMoveShouldSetPanResponder: (event: unknown, gesture: Gesture) => boolean;
  onPanResponderMove: (event: unknown, gesture: Gesture) => void;
  onPanResponderRelease: (event: unknown, gesture: Gesture) => void;
  onPanResponderTerminate: () => void;
};

function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

function setup(
  reduced = false,
  dimensions = { width: 390, height: 844 },
  insetTop = 47,
  deferAnimation = false,
  autoLayout = true,
  motionReady = true,
) {
  let preference: boolean | undefined = motionReady ? reduced : undefined;
  let handlers!: PanHandlers;
  let closeAttempts = 0;
  let allowedCloses = 0;
  let canClose = true;
  let guard: (() => Promise<boolean>) | undefined;
  let onClosed: (() => void) | undefined;
  let hardwareBack = () => false;
  const controller: { current: WorkspaceSheetHandle | null } = {
    current: null,
  };
  const springs: {
    toValue: number;
    useNativeDriver: boolean;
    started: boolean;
  }[] = [];
  const timings: {
    from: number;
    to: number;
    value: Value;
    complete: () => void;
  }[] = [];
  class Value {
    constructor(public value: number) {}
    setValue(next: number) {
      this.value = next;
    }
  }
  const child: Element = { type: "ChildNavigator", props: {} };
  const jsx = (type: unknown, props: Record<string, unknown>): Element => ({
    type,
    props,
  });
  const harness = hookHarness<Element>({
    "../../lib/accessibility/workspaceAccessibility": {
      useWorkspaceAccessibility: () => ({
        primaryKey: "chat",
        sheetOpen: true,
      }),
    },
    "../../lib/accessibility/workspaceFocus": {
      workspaceFocus: { capture() {}, restore() {} },
    },
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotionPreference: () => preference,
    },
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      View: "View",
      InteractionManager: {
        runAfterInteractions: (callback: () => void) => {
          callback();
          return { cancel() {} };
        },
      },
      useWindowDimensions: () => dimensions,
      BackHandler: {
        addEventListener: (_: string, callback: () => boolean) => {
          hardwareBack = callback;
          return { remove() {} };
        },
      },
      AccessibilityInfo: { setAccessibilityFocus() {} },
      findNodeHandle: () => null,
      Pressable: "Pressable",
      Animated: {
        Value,
        timing: (value: Value, config: { toValue: number }) => ({
          start: (done?: (event: { finished: boolean }) => void) => {
            const complete = () => {
              value.setValue(config.toValue);
              done?.({ finished: true });
            };
            timings.push({
              from: value.value,
              to: config.toValue,
              value,
              complete,
            });
            if (!deferAnimation) complete();
          },
          stop() {},
        }),
        View: "Animated.View",
        spring: (
          value: Value,
          config: { toValue: number; useNativeDriver: boolean },
        ) => {
          const entry = {
            toValue: config.toValue,
            useNativeDriver: config.useNativeDriver,
            started: false,
          };
          springs.push(entry);
          return {
            start: () => {
              entry.started = true;
              value.setValue(config.toValue);
            },
          };
        },
      },
      PanResponder: {
        create: (next: PanHandlers) => {
          handlers = next;
          return { panHandlers: next };
        },
      },
      StyleSheet: {
        create: <T>(styles: T) => styles,
        absoluteFill: { position: "absolute" },
      },
    },
    "react-native-safe-area-context": {
      SafeAreaProvider: "SafeAreaProvider",
      useSafeAreaInsets: () => ({
        top: insetTop,
        bottom: 34,
        left: 0,
        right: 0,
      }),
    },
    "../../theme/ThemeContext": {
      SheetThemeProvider: "SheetThemeProvider",
      useTheme: () => ({
        colors: {
          bg: "#ffffff",
          sheetScrim: "rgba(0,0,0,0.42)",
          textMuted: "#746981",
        },
      }),
    },
    "@react-navigation/native": {
      useNavigation: () => ({
        goBack: () => {
          throw new Error("Surface must dismiss through guarded onClose");
        },
        dispatch: () => {
          throw new Error("Surface must dismiss through guarded onClose");
        },
      }),
    },
  });
  const component = harness.load(
    fileURLToPath(new URL("./WorkspaceSheetSurface.tsx", import.meta.url)),
  ).WorkspaceSheetSurface as (props: {
    onClose: () => void;
    canClose: () => Promise<boolean>;
    controller: typeof controller;
    children: Element;
  }) => Element;
  harness.mount(() =>
    component({
      children: child,
      controller,
      canClose: async () => {
        closeAttempts++;
        return guard ? guard() : canClose;
      },
      onClose: () => {
        allowedCloses++;
        onClosed?.();
      },
    }),
  );
  if (autoLayout) harness.value().props.onLayout();
  const all = () => elements(harness.value());
  const panel = () => {
    const value = all().find(
      (element) => element.props.accessibilityLabel === "Workspace drawer",
    );
    assert.ok(value);
    return value;
  };
  const offset = () => panel().props.style[1].transform[0].translateY as Value;
  const gesture = (dy: number, vy = 0, dx = 0): Gesture => ({ dx, dy, vy });
  return {
    panel,
    rotate: (width: number, height: number, top: number) => {
      dimensions = { width, height };
      insetTop = top;
      harness.render();
    },
    present: () => harness.value().props.onLayout(),
    resolveMotion: () => {
      preference = reduced;
      harness.render();
    },
    harness,
    handlers: () => handlers,
    all,
    child,
    offset,
    springs,
    timings,
    gesture,
    counts: () => ({ closeAttempts, allowedCloses }),
    controller,
    hardwareBack: () => hardwareBack(),
    guardWith: (next: () => Promise<boolean>) => {
      guard = next;
    },
    onClosed: (next: () => void) => {
      onClosed = next;
    },
    blockClose: () => {
      canClose = false;
    },
    backdrop: () => {
      const value = all().find(
        (element) => element.props.accessibilityLabel === "Dismiss drawer",
      );
      assert.ok(value);
      return value;
    },
  };
}

test("the handle claims a deliberate downward drag without claiming taps, horizontal, or upward movement", () => {
  const env = setup();
  const handlers = env.handlers();
  assert.equal("onStartShouldSetPanResponder" in handlers, false);
  for (const gesture of [
    env.gesture(0),
    env.gesture(8),
    env.gesture(-40),
    env.gesture(20, 0, 30),
    env.gesture(20, 0, -20),
  ]) {
    assert.equal(handlers.onMoveShouldSetPanResponder(null, gesture), false);
  }
  assert.equal(
    handlers.onMoveShouldSetPanResponder(null, env.gesture(20, 0, 4)),
    true,
  );
  assert.deepEqual(env.counts(), { closeAttempts: 0, allowedCloses: 0 });
});

test("the management scrim fades independently of the sliding panel and stays opaque when Close is blocked", async () => {
  const env = setup();
  const scrim = env
    .all()
    .find(
      (node) =>
        node.type === "Animated.View" && node.props.pointerEvents === "none",
    );
  assert.ok(scrim);
  const scrimStyle = scrim.props.style[1];
  assert.equal(
    scrimStyle.transform,
    undefined,
    "the background never translates with the sheet",
  );
  assert.equal(env.timings[0]?.from, 844);
  assert.equal(env.timings[0]?.to, 0);
  assert.equal(env.timings[1]?.from, 0);
  assert.equal(env.timings[1]?.to, 1);
  assert.equal(env.timings[1]?.value, scrimStyle.opacity);
  assert.notEqual(scrimStyle.opacity, env.offset());
  env.blockClose();
  env.backdrop().props.onPress();
  await env.harness.settle();
  assert.equal(scrimStyle.opacity.value, 1);
  assert.equal(
    env.timings.length,
    2,
    "a dirty or pending guard cannot fade out the background",
  );
  env.harness.unmount();

  const allowed = setup();
  allowed.backdrop().props.onPress();
  await allowed.harness.settle();
  assert.deepEqual(
    allowed.timings.slice(2).map(({ from, to }) => [from, to]),
    [
      [1, 0],
      [0, 844],
    ],
  );
  assert.deepEqual(allowed.counts(), { closeAttempts: 1, allowedCloses: 1 });
  allowed.harness.unmount();
});

test("an early Close retains the page until its slide finishes and a late animation cannot close an unmounted surface", async () => {
  const env = setup(false, { width: 390, height: 844 }, 47, true);
  env.backdrop().props.onPress();
  env.backdrop().props.onPress();
  await env.harness.settle();
  assert.deepEqual(env.counts(), { closeAttempts: 1, allowedCloses: 0 });
  const [fade, slide] = env.timings.slice(2);
  assert.ok(fade && slide);
  fade.complete();
  assert.equal(env.counts().allowedCloses, 0);
  slide.complete();
  assert.equal(env.counts().allowedCloses, 1);
  env.harness.unmount();

  const removed = setup(false, { width: 390, height: 844 }, 47, true);
  removed.backdrop().props.onPress();
  await removed.harness.settle();
  removed.harness.unmount();
  removed.timings.forEach((animation) => animation.complete());
  assert.equal(removed.counts().allowedCloses, 0);
});

test("gesture ownership is confined to the handle so nested pages keep their own scrolling", () => {
  const env = setup();
  const owners = env
    .all()
    .filter((element) => element.props.onMoveShouldSetPanResponder);
  assert.equal(owners.length, 1);
  assert.equal(owners[0]?.type, "View");
  assert.equal(owners[0]?.props.accessible, false);
  assert.equal(
    env.all().find((element) => element.type === "ChildNavigator"),
    env.child,
  );
  assert.equal("onMoveShouldSetPanResponder" in env.child.props, false);
});

test("a short drag springs back without asking to close the drawer", () => {
  const env = setup();
  env.handlers().onPanResponderMove(null, env.gesture(45));
  assert.equal(env.offset().value, 45);
  env.handlers().onPanResponderRelease(null, env.gesture(45, 0.2));
  assert.equal(env.offset().value, 0);
  assert.deepEqual(env.springs, [
    { toValue: 0, useNativeDriver: true, started: true },
  ]);
  assert.deepEqual(env.counts(), { closeAttempts: 0, allowedCloses: 0 });
});

test("a cancelled drag always springs back, including past the dismissal threshold", () => {
  const env = setup();
  env.handlers().onPanResponderMove(null, env.gesture(120));
  assert.equal(env.offset().value, 120);
  env.handlers().onPanResponderTerminate();
  assert.equal(env.offset().value, 0);
  assert.equal(env.springs.at(-1)?.started, true);
  assert.deepEqual(env.counts(), { closeAttempts: 0, allowedCloses: 0 });
  env.handlers().onPanResponderMove(null, env.gesture(-30));
  assert.equal(
    env.offset().value,
    0,
    "dragging back upward never moves the panel above its resting position",
  );
});

test("distance or downward velocity asks guarded onClose exactly once for each released gesture", async () => {
  for (const release of [
    { dy: 81, vy: 0 },
    { dy: 15, vy: 0.9 },
  ]) {
    const env = setup();
    env.handlers().onPanResponderMove(null, env.gesture(release.dy));
    env
      .handlers()
      .onPanResponderRelease(null, env.gesture(release.dy, release.vy));
    await env.harness.settle();
    assert.deepEqual(env.counts(), { closeAttempts: 1, allowedCloses: 1 });
    assert.equal(env.offset().value, 844);
    assert.equal(env.springs.length, 0);
  }
  const boundary = setup();
  boundary.handlers().onPanResponderRelease(null, boundary.gesture(80, 0.8));
  assert.equal(boundary.counts().closeAttempts, 0);
});

test("a navigation guard rejecting a drag dismissal leaves the panel at rest and mounted", async () => {
  const env = setup();
  env.blockClose();
  env.handlers().onPanResponderMove(null, env.gesture(150));
  env.handlers().onPanResponderRelease(null, env.gesture(150, 1));
  await env.harness.settle();
  assert.deepEqual(env.counts(), { closeAttempts: 1, allowedCloses: 0 });
  assert.equal(env.offset().value, 0);
  assert.equal(env.all().includes(env.child), true);
});

test("the accessible backdrop delegates dismissal through the same guarded onClose", async () => {
  const env = setup();
  assert.equal(env.backdrop().props.accessibilityRole, "button");
  env.blockClose();
  env.backdrop().props.onPress();
  await env.harness.settle();
  assert.deepEqual(env.counts(), { closeAttempts: 1, allowedCloses: 0 });
  assert.equal(env.all().includes(env.child), true);
  assert.equal(env.springs.length, 1);
});

test("drawer geometry clears the top safe area at small, large, short and wide native sizes", () => {
  for (const [width, height, top] of [
    [320, 568, 20],
    [390, 844, 47],
    [430, 932, 59],
    [640, 360, 24],
    [1024, 768, 24],
  ]) {
    const env = setup(false, { width: width!, height: height! }, top!);
    assert.equal(env.panel().props.style[1].height, height! - top! - 10);
    const handle = env
      .all()
      .find((node) => node.props.onMoveShouldSetPanResponder);
    assert.ok(handle);
    assert.equal(handle.props.style.height, 44);
    assert.equal(env.panel().props.style[0].borderTopLeftRadius, 24);
    env.harness.unmount();
  }
});
test("reduced motion dismisses without travelling or springing and accessibility escape shares the guard", async () => {
  const env = setup(true);
  env.handlers().onPanResponderMove(null, env.gesture(150));
  assert.equal(env.offset().value, 0);
  env.blockClose();
  env.all()[0]!.props.onAccessibilityEscape();
  await env.harness.settle();
  assert.deepEqual(env.counts(), { closeAttempts: 1, allowedCloses: 0 });
  assert.equal(env.springs.length, 0);
  assert.equal(env.all()[0]!.props.accessibilityViewIsModal, true);
});

for (const source of ["Close", "backdrop", "drag", "Android Back", "Escape"]) {
  test(`dirty ${source} confirms through the surface, cancels responsively and discards to the same chat`, async () => {
    const env = setup();
    const guards = new Map<symbol, () => Promise<boolean>>();
    const alerts: {
      title: string;
      buttons: { text: string; onPress?: () => void }[];
    }[] = [];
    let header: ReturnType<typeof workspaceHeaderOptions> | undefined;
    const headerHarness = hookHarness<null>({
      "react/jsx-runtime": {
        jsx: (type: unknown, props: Record<string, unknown>) => ({
          type,
          props,
        }),
      },
      "react-native": {
        Alert: {
          alert: (
            title: string,
            _message: string,
            buttons: (typeof alerts)[number]["buttons"],
          ) => alerts.push({ title, buttons }),
        },
        BackHandler: { addEventListener: () => ({ remove() {} }) },
        StyleSheet: { create: <T>(styles: T) => styles },
      },
      "@react-navigation/native": {
        useIsFocused: () => true,
        useNavigation: () => ({
          setOptions: (next: typeof header) => {
            header = next;
          },
        }),
        usePreventRemove() {},
      },
      "lucide-react-native": {},
      "../lib/accessibility/WorkspaceInteractionSurface": {},
      "../lib/accessibility/workspaceAccessibility": {},
      "../lib/accessibility/workspaceFocus": {},
      "../theme/ThemeContext": {},
    });
    const module = headerHarness.load(
      fileURLToPath(new URL("../../app/WorkspaceHeader.tsx", import.meta.url)),
    ) as {
      useWorkspaceBack: typeof useWorkspaceBack;
      WorkspaceSheetContext: { value: unknown };
    };
    module.WorkspaceSheetContext.value = {
      registerDismissGuard: (id: symbol, guard: () => Promise<boolean>) => {
        guards.set(id, guard);
        return () => {
          guards.delete(id);
        };
      },
      rootKey: "root",
      sheetKey: "sheet",
      onClose: () => env.controller.current!.dismiss(),
    };
    headerHarness.mount(() => {
      module.useWorkspaceBack({
        active: true,
        onBack: () => {
          throw new Error("Outer dismissal must not invoke local Back");
        },
        discardOnClose: { title: "Discard draft?", message: "Unsaved changes" },
      });
      return null;
    });
    env.guardWith(async () => {
      for (const guard of guards.values()) if (!(await guard())) return false;
      return true;
    });
    const chat = {
      key: "exact-chat",
      name: "AssistantContext",
      params: { conversationId: "conversation-72", agentId: "researcher" },
    };
    const state = {
      key: "root",
      type: "stack",
      stale: false as const,
      index: 2,
      routeNames: ["Workspace", "AssistantContext", "WorkspaceSheet"],
      routes: [
        { key: "workspace", name: "Workspace" },
        chat,
        { key: "sheet", name: "WorkspaceSheet" },
      ],
    };
    const closedActions: NonNullable<
      ReturnType<typeof dismissWorkspaceSheetAction>
    >[] = [];
    env.onClosed(() => {
      const action = dismissWorkspaceSheetAction(state);
      assert.ok(action);
      closedActions.push(action);
    });
    const close = () => {
      const button = header?.headerRight?.({
        canGoBack: true,
      }) as unknown as Element;
      assert.ok(button);
      button.props.onPress();
    };
    const attempt = () => {
      if (source === "Close") close();
      else if (source === "backdrop") env.backdrop().props.onPress();
      else if (source === "drag") {
        env.handlers().onPanResponderMove(null, env.gesture(150));
        env.handlers().onPanResponderRelease(null, env.gesture(150, 1));
      } else if (source === "Android Back")
        assert.equal(env.hardwareBack(), true);
      else env.all()[0]!.props.onAccessibilityEscape();
    };
    const choose = (text: string) => {
      const button = alerts.at(-1)?.buttons.find((row) => row.text === text);
      assert.ok(button);
      button.onPress?.();
    };
    attempt();
    await env.harness.settle();
    assert.equal(alerts.length, 1);
    assert.equal(alerts[0]?.title, "Discard draft?");
    assert.equal(env.counts().allowedCloses, 0);
    assert.equal(
      env.timings.length,
      2,
      "no exit animation before confirmation",
    );
    close();
    assert.equal(
      alerts.length,
      1,
      "an outstanding confirmation is not duplicated",
    );
    choose("Keep editing");
    await env.harness.settle();
    assert.equal(env.panel().props.pointerEvents, "auto");
    assert.equal(env.offset().value, 0);
    assert.equal(env.all().includes(env.child), true);
    assert.equal(closedActions.length, 0);
    assert.deepEqual(env.counts(), { closeAttempts: 1, allowedCloses: 0 });
    attempt();
    await env.harness.settle();
    assert.equal(alerts.length, 2, "cancellation releases the closing lock");
    choose("Discard");
    await env.harness.settle();
    assert.deepEqual(env.counts(), { closeAttempts: 2, allowedCloses: 1 });
    assert.equal(closedActions.length, 1);
    const closedAction = closedActions[0]!;
    assert.ok(closedAction.payload);
    assert.equal(closedAction.payload.index, 1);
    assert.equal(closedAction.payload.routes[1], chat);
    assert.equal(closedAction.target, state.key);
    headerHarness.unmount();
    env.harness.unmount();
  });
}

test("a sheet does not consume its entrance before native layout and motion readiness", () => {
  const x = setup(false, { width: 390, height: 844 }, 47, false, false, false);
  assert.equal(x.timings.length, 0);
  assert.equal(x.offset().value, 844);
  x.present();
  assert.equal(x.timings.length, 0);
  x.resolveMotion();
  assert.equal(x.timings.length, 2);
  assert.deepEqual(
    x.timings.map((t) => [t.from, t.to]),
    [
      [844, 0],
      [0, 1],
    ],
  );
  x.present();
  assert.equal(x.timings.length, 2);
  x.harness.unmount();
});
for (const entry of [
  "Settings",
  "Account & security",
  "Usage & credits",
  "Scan QR code",
  "Terminal login code",
]) {
  test(`${entry} repeats a full entrance and guarded exit over the unchanged sidebar`, async () => {
    for (let presentation = 0; presentation < 2; presentation++) {
      const x = setup(false, { width: 390, height: 844 }, 47, true, false);
      assert.equal(x.timings.length, 0);
      x.present();
      assert.equal(x.timings[0]!.from, 844);
      x.timings[0]!.complete();
      x.timings[1]!.complete();
      x.controller.current!.dismiss();
      await x.harness.settle();
      assert.equal(x.counts().allowedCloses, 0);
      assert.equal(x.timings.at(-1)!.to, 844);
      x.timings.at(-1)!.complete();
      assert.equal(x.counts().allowedCloses, 1);
      x.harness.unmount();
    }
  });
}

test("a sheet follows landscape dimensions and safe areas, then dismisses below the restored portrait screen", async () => {
  const x = setup();
  assert.equal(x.panel().props.style[1].height, 787);
  x.rotate(844, 390, 0);
  assert.equal(x.panel().props.style[1].height, 380);
  let provider = x.all().find((node) => node.type === "SafeAreaProvider")!;
  assert.deepEqual({ ...provider.props.initialMetrics.frame }, { x: 0, y: 10, width: 844, height: 380 });
  assert.equal(x.offset().value, 0);
  x.rotate(390, 844, 47);
  assert.equal(x.panel().props.style[1].height, 787);
  provider = x.all().find((node) => node.type === "SafeAreaProvider")!;
  assert.deepEqual({ ...provider.props.initialMetrics.frame }, { x: 0, y: 57, width: 390, height: 787 });
  assert.ok(x.all().includes(x.child), "rotation keeps the sheet's navigator mounted");
  x.controller.current!.dismiss();
  await x.harness.settle();
  assert.equal(x.timings.at(-1)?.to, 844);
  assert.equal(x.counts().allowedCloses, 1);
});
