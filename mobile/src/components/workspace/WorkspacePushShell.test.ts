import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import {
  clampSidebarOffset,
  settleSidebarOpen,
  sidebarPushWidth,
  SIDEBAR_EDGE_WIDTH,
  workspaceSidebarLayout,
} from "./workspacePushState";

type Element = { type: unknown; props: Record<string, any> };
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
function setup(reduced = false, width = 402, height = 874, platform = "ios") {
  let dimensions = { width, height };
  let insets = { top: 0, right: 0, bottom: 0, left: 0 };
  let railHidden = false;
  let statusOffset = 0;
  const railWrites: boolean[] = [];
  const layoutAnimations: unknown[] = [];
  let props: Record<string, any>;
  const shared: { value: number }[] = [];
  let sharedCursor = 0;
  const springs: { target: number; complete: (done: boolean) => void }[] = [];
  const timings: {
    target: number;
    config: any;
    complete: (done: boolean) => void;
  }[] = [];
  const focus: unknown[][] = [];
  let reaction: {
    read: () => any;
    changed: (value: any, previous: any) => void;
  };
  let previousEndpoint: any;
  let hardwareBack: (() => boolean) | undefined;
  let dismissed = 0;
  let reveals = 0;
  const contentTransition = { current: null as any };
  const changes: boolean[] = [];
  let handlers: Record<string, any> = {};
  const settings: Record<string, any> = {};
  const child = {
    type: "ActualMountedNavigator",
    props: { key: "stable-chat" },
  };
  const sidebar = { type: "Sidebar", props: {} };
  const jsx = (type: unknown, props: Record<string, unknown>) => ({
    type,
    props,
  });
  const pan = new Proxy(
    {},
    {
      get: (_, key: string) => (value: unknown) => {
        (key.startsWith("on") ? handlers : settings)[key] = value;
        return pan;
      },
    },
  );
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Platform: { OS: platform },
      View: "View",
      Pressable: "Pressable",
      useWindowDimensions: () => dimensions,
      LayoutAnimation: {
        Presets: { easeInEaseOut: "easeInEaseOut" },
        configureNext: (config: unknown) => layoutAnimations.push(config),
      },
      StyleSheet: {
        create: (styles: unknown) => styles,
        absoluteFill: {
          position: "absolute",
          top: 0,
          bottom: 0,
          left: 0,
          right: 0,
        },
      },
      BackHandler: {
        addEventListener: (_: string, handler: () => boolean) => {
          hardwareBack = handler;
          return {
            remove: () => {
              if (hardwareBack === handler) hardwareBack = undefined;
            },
          };
        },
      },
    },
    "react-native-safe-area-context": {
      useSafeAreaInsets: () => insets,
    },
    "./useWorkspaceStatusOffset": {
      useWorkspaceStatusOffset: () => statusOffset,
    },
    "./WorkspaceKeyboardFocusBoundary": {
      WorkspaceKeyboardFocusBoundary: "WorkspaceKeyboardFocusBoundary",
    },
    "./workspaceQuickRailPreference": {
      useQuickRailHidden: () => [
        railHidden,
        (hidden: boolean) => {
          railWrites.push(hidden);
          railHidden = hidden;
        },
      ],
    },
    "expo-linear-gradient": { LinearGradient: "LinearGradient" },
    "react-native-gesture-handler": {
      Gesture: {
        Pan: () => {
          handlers = {};
          return pan;
        },
      },
      GestureDetector: "GestureDetector",
    },
    "react-native-reanimated": {
      default: { View: "Animated.View" },
      useSharedValue: (value: number) => {
        const index = sharedCursor++;
        return (shared[index] ??= { value });
      },
      useAnimatedStyle: (read: () => unknown) => ({ read }),
      useAnimatedReaction: (
        read: () => any,
        changed: (value: any, previous: any) => void,
      ) => {
        reaction = { read, changed };
      },
      cancelAnimation() {},
      runOnJS: (callback: unknown) => callback,
      Easing: { bezier: (...points: number[]) => points },
      withTiming: (
        target: number,
        config: unknown,
        complete: (done: boolean) => void,
      ) => {
        timings.push({ target, config, complete });
        return target;
      },
      withSpring: (
        target: number,
        _config: unknown,
        complete: (done: boolean) => void,
      ) => {
        springs.push({ target, complete });
        return target;
      },
    },
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => reduced,
    },
    "../../lib/accessibility/workspaceFocus": {
      workspaceFocus: {
        setModalActive: (owner: symbol, active: boolean) =>
          focus.push(["active", owner, active]),
        focusModalOnce: (owner: symbol, target: () => number | null) =>
          focus.push(["focus", owner, target()]),
      },
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: { sidebarBase: "#0C0C0E", bg: "#17171A", shadowColor: "#000" },
      }),
    },
    "./workspacePushState": {
      clampSidebarOffset,
      settleSidebarOpen,
      sidebarPushWidth,
      SIDEBAR_EDGE_WIDTH,
      workspaceSidebarLayout,
    },
  });
  const component = harness.load(
    fileURLToPath(new URL("./WorkspacePushShell.tsx", import.meta.url)),
  ).WorkspacePushShell as (props: any) => Element;
  props = {
    contentTransition,
    enabled: true,
    visible: false,
    presented: false,
    gestureEnabled: true,
    children: child,
    sidebar,
    onOpenChange: (open: boolean) => {
      changes.push(open);
      props.visible = open;
    },
    onRevealStart: () => {
      reveals++;
      props.presented = true;
    },
    onDismiss: () => {
      dismissed++;
      props.presented = false;
    },
    initialFocusTarget: () => 99,
  };
  harness.mount(() => {
    sharedCursor = 0;
    return component({ ...props });
  });
  const all = () => elements(harness.value());
  const find = (id: string) => all().find((node) => node.props.testID === id)!;
  const render = () => harness.render();
  const set = (next: Record<string, any>) => {
    Object.assign(props, next);
    render();
  };
  return {
    harness,
    all,
    find,
    set,
    status: (offset: number) => {
      statusOffset = offset;
      render();
    },
    rotate: (width: number, height: number) => {
      dimensions = { width, height };
      render();
    },
    window: (
      width: number,
      height: number,
      next: Partial<typeof insets> = {},
    ) => {
      dimensions = { width, height };
      insets = { top: 0, right: 0, bottom: 0, left: 0, ...next };
      render();
    },
    railWrites,
    layoutAnimations,
    render,
    shared,
    springs,
    timings,
    focus,
    changes,
    contentTransition,
    child,
    sidebar,
    settings,
    handlers: () => handlers,
    offset: () => shared[0]!.value,
    opacity: () => find("workspace-slab-content").props.style[1].read().opacity,
    style: () => find("workspace-content-slab").props.style[2].read(),
    lastComplete: (done = true) => {
      timings.at(-1)!.complete(done);
      harness.render();
    },
    springComplete: (done = true) => {
      springs.at(-1)!.complete(done);
      harness.render();
    },
    reconcile: () => {
      const value = reaction.read();
      reaction.changed(value, previousEndpoint);
      previousEndpoint = value;
      harness.render();
    },
    counts: () => ({ dismissed, reveals }),
    hardwareBack: () => hardwareBack?.(),
  };
}

test("the stationary base and full-width translated slab retain the actual navigator without a scrim or modal", () => {
  const x = setup();
  assert.equal(x.find("workspace-sidebar-base").props.style[1].width, 310);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "none");
  assert.ok(x.all().includes(x.child));
  assert.ok(x.all().includes(x.sidebar));
  assert.equal(x.find("workspace-content-slab").props.style[0].flex, 1);
  assert.equal(
    x.all().filter((node) => /Modal|Overlay|Scrim/.test(String(node.type)))
      .length,
    0,
  );
  x.set({ visible: true, presented: true });
  assert.equal(x.style().transform[0].translateX, 310);
  assert.equal(x.style().borderTopLeftRadius, 16);
  assert.equal(
    x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
    true,
    "opening travel does not expose an invisible sidebar",
  );
  x.lastComplete();
  assert.equal(
    x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
    false,
  );
  assert.equal(
    x.find("workspace-sidebar-close-sliver").props.accessibilityLabel,
    "Close navigation",
  );
  assert.equal(x.find("workspace-sidebar-close-sliver").props.focusable, true);
  assert.equal(
    x.find("workspace-sidebar-close-sliver").props.importantForAccessibility,
    "yes",
  );
  assert.ok(x.all().includes(x.child));
  x.harness.unmount();
});

test("Android excludes the closed compact drawer from native focus search and shows it throughout the reveal and dismissal", () => {
  const x = setup(false, 402, 874, "android");
  const sidebarStyle = () =>
    Object.assign({}, ...x.find("workspace-sidebar-base").props.style);
  const boundary = () =>
    x.all().find((node) => node.type === "WorkspaceKeyboardFocusBoundary")!;

  assert.equal(sidebarStyle().display, "none");
  assert.equal(boundary().props.blocked, false);
  assert.ok(elements(boundary()).includes(x.child));
  assert.ok(!elements(boundary()).includes(x.sidebar));
  assert.ok(x.all().includes(x.sidebar));
  assert.ok(x.all().includes(x.child));

  x.handlers().onStart();
  x.render();
  assert.equal(sidebarStyle().display, undefined);
  assert.equal(boundary().props.blocked, true);
  x.handlers().onUpdate({ translationX: 240 });
  x.handlers().onEnd({ velocityX: 0 });
  x.render();
  x.springComplete();
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  assert.equal(boundary().props.blocked, true);
  assert.ok(!elements(boundary()).includes(x.find("workspace-sidebar-close-sliver")));

  x.find("workspace-sidebar-close-sliver").props.onPress();
  x.render();
  assert.equal(sidebarStyle().display, undefined);
  assert.equal(boundary().props.blocked, true);
  x.lastComplete();
  assert.equal(sidebarStyle().display, "none");
  assert.equal(boundary().props.blocked, false);

  x.rotate(1194, 834);
  assert.equal(sidebarStyle().display, undefined);
  assert.equal(boundary().props.blocked, false);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  x.harness.unmount();
});

test("edge pan follows the finger, clamps travel and settles at the projected threshold", () => {
  const x = setup();
  assert.equal(x.settings.activeOffsetX, 18);
  assert.deepEqual([...x.settings.failOffsetY], [-10, 10]);
  assert.equal(x.find("workspace-sidebar-reveal-edge").props.style.width, 24);
  const edge = x.find("workspace-sidebar-reveal-edge");
  assert.equal(edge.props.accessible, false);
  assert.equal(edge.props.focusable, false);
  assert.equal(edge.props.accessibilityElementsHidden, true);
  assert.equal(edge.props.importantForAccessibility, "no-hide-descendants");
  const detector = x.all().find((node) => node.type === "GestureDetector")!;
  assert.equal(
    detector.props.children.props.testID,
    "workspace-sidebar-reveal-edge",
  );
  assert.ok(
    !elements(detector).includes(x.child),
    "no ancestor pan on content",
  );
  x.handlers().onStart();
  x.render();
  x.handlers().onUpdate({ translationX: 155 });
  assert.equal(x.offset(), 155);
  x.handlers().onUpdate({ translationX: 600 });
  assert.equal(x.offset(), 310);
  x.handlers().onUpdate({ translationX: 155 });
  x.handlers().onEnd({ velocityX: 0 });
  x.render();
  assert.deepEqual(x.changes, [true]);
  x.springComplete();
  assert.equal(
    x.springs.length,
    1,
    "one settle, no timing restarted by the controlled commit",
  );
  assert.equal(x.focus.filter((event) => event[0] === "focus").length, 1);
  x.handlers().onStart();
  x.render();
  x.handlers().onUpdate({ translationX: -500 });
  assert.equal(x.offset(), 0);
  x.handlers().onEnd({ velocityX: -100 });
  x.render();
  assert.equal(x.counts().dismissed, 0);
  x.springComplete();
  assert.equal(x.counts().dismissed, 1);
  x.harness.unmount();
});

test("gesture cancellation restores the previous endpoint and disabling reveal preserves the content", () => {
  const x = setup();
  x.set({ gestureEnabled: false });
  assert.equal(x.settings.enabled, false);
  x.set({ gestureEnabled: true });
  x.handlers().onStart();
  x.render();
  x.handlers().onUpdate({ translationX: 160 });
  x.handlers().onFinalize({}, false);
  x.render();
  assert.deepEqual(x.changes, [false]);
  x.springComplete();
  assert.equal(x.counts().dismissed, 1);
  assert.ok(x.all().includes(x.child));
  x.harness.unmount();
});

test("sliver, Escape and Android Back request the same close; only settled current transitions dismiss", () => {
  const x = setup();
  x.set({ visible: true, presented: true });
  x.lastComplete();
  x.find("workspace-sidebar-close-sliver").props.onPress();
  x.render();
  const staleClose = x.timings.at(-1)!;
  assert.equal(x.counts().dismissed, 0);
  x.set({ visible: true });
  staleClose.complete(true);
  assert.equal(x.counts().dismissed, 0);
  x.harness.value().props.onAccessibilityEscape();
  x.render();
  x.lastComplete(false);
  assert.equal(x.counts().dismissed, 0);
  x.set({ visible: true });
  assert.equal(x.hardwareBack(), true);
  x.render();
  x.lastComplete();
  x.render();
  assert.equal(x.counts().dismissed, 1);
  assert.deepEqual(x.changes, [false, false, false]);
  assert.equal(x.hardwareBack(), undefined);
  assert.equal(
    x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
    true,
  );
  x.harness.unmount();
});

test("sign-out and unmount fence old animation callbacks and release focus ownership", () => {
  for (const unmount of [false, true]) {
    const x = setup();
    x.set({ visible: true, presented: true });
    x.set({ visible: false });
    const pending = x.timings.at(-1)!;
    if (unmount) x.harness.unmount();
    else x.set({ enabled: false });
    pending.complete(true);
    assert.equal(x.counts().dismissed, 0);
    assert.equal(x.focus.filter((event) => event[0] === "focus").length, 0);
    assert.equal(x.focus.at(-1)![2], false);
    if (!unmount) {
      assert.equal(x.offset(), 0);
      x.harness.unmount();
    }
  }
});

test("reduced motion uses instant endpoints without a spring or interactive travel", async () => {
  const x = setup(true);
  x.handlers().onStart();
  x.render();
  x.handlers().onUpdate({ translationX: 180 });
  assert.equal(x.offset(), 0);
  x.handlers().onEnd({ translationX: 180, velocityX: 0 });
  x.render();
  assert.equal(x.offset(), 310);
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(x.focus.filter((event) => event[0] === "focus").length, 1);
  x.find("workspace-sidebar-close-sliver").props.onPress();
  x.render();
  assert.equal(x.offset(), 0);
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(x.counts().dismissed, 1);
  assert.equal(x.springs.length, 0);
  assert.equal(x.timings.length, 0);
  x.harness.unmount();
});

test("a nested dirty editor edge swipe requests guarded Back without translating or exposing the sidebar", () => {
  const x = setup();
  let backs = 0;
  x.set({ onEdgeBack: () => backs++ });
  x.handlers().onStart();
  x.handlers().onUpdate({ translationX: 160 });
  assert.equal(x.offset(), 0);
  assert.equal(x.counts().reveals, 0);
  x.handlers().onEnd({ translationX: 160, velocityX: 0 });
  assert.equal(backs, 1);
  assert.deepEqual(x.changes, []);
  x.handlers().onStart();
  x.handlers().onEnd({ translationX: 15, velocityX: 0 });
  assert.equal(backs, 1, "a short or cancelled drag keeps editing");
  x.handlers().onStart();
  x.handlers().onFinalize({}, false);
  assert.equal(backs, 1);
  x.harness.unmount();
});

test("programmatic travel uses one 300ms curve and rejects pans until its current completion", () => {
  const x = setup();
  x.set({ visible: true, presented: true });
  assert.equal(x.timings.length, 1);
  assert.equal(x.timings[0]!.config.duration, 300);
  assert.deepEqual([...x.timings[0]!.config.easing], [0.2, 0.8, 0.2, 1]);
  assert.equal(x.settings.activeOffsetX, -18);
  const detector = x.all().find((node) => node.type === "GestureDetector")!;
  assert.equal(
    detector.props.children.props.testID,
    "workspace-sidebar-close-sliver",
  );
  let fails = 0;
  x.handlers().onTouchesDown({}, { fail: () => fails++ });
  x.handlers().onStart();
  x.handlers().onUpdate({ translationX: -200 });
  x.handlers().onEnd({ translationX: -200, velocityX: -700 });
  assert.equal(fails, 1);
  assert.equal(x.offset(), 310);
  assert.equal(x.springs.length, 0);
  x.lastComplete();
  x.find("workspace-sidebar-close-sliver").props.onPress();
  x.render();
  x.render();
  assert.equal(
    x.timings.length,
    2,
    "one close animation despite repeated commits",
  );
  x.lastComplete();
  assert.equal(x.counts().dismissed, 1);
  x.harness.unmount();
});

test("Settings owns input and Back above the shell without changing its open offset or mounted sidebar", () => {
  const x = setup();
  x.set({ visible: true, presented: true });
  x.lastComplete();
  const timings = x.timings.length;
  x.set({ obscured: true });
  assert.equal(x.offset(), 310);
  assert.equal(x.harness.value().props.pointerEvents, "none");
  assert.equal(x.harness.value().props.accessibilityElementsHidden, true);
  assert.equal(
    x.hardwareBack(),
    undefined,
    "Settings handles Android Back first",
  );
  assert.ok(x.all().includes(x.sidebar));
  assert.equal(
    x.all().some((node) => node.type === "GestureDetector"),
    false,
  );
  x.harness.value().props.onAccessibilityEscape();
  assert.deepEqual(x.changes, []);
  x.set({ obscured: false });
  assert.equal(x.offset(), 310);
  assert.equal(x.timings.length, timings);
  assert.ok(x.all().includes(x.sidebar));
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  x.harness.unmount();
});

test("content opacity has separate 120ms out and 180ms in phases; the slab stays opaque", () => {
  const x = setup();
  let swaps = 0;
  x.set({ visible: true, presented: true });
  x.lastComplete();
  x.contentTransition.current.fadeOut(() => swaps++);
  assert.equal(swaps, 0);
  assert.equal(x.timings.at(-1)!.target, 0);
  assert.equal(x.timings.at(-1)!.config.duration, 120);
  x.lastComplete();
  assert.equal(swaps, 1);
  x.contentTransition.current.reveal();
  assert.equal(x.timings.at(-1)!.target, 1);
  assert.equal(x.timings.at(-1)!.config.duration, 180);
  assert.equal(
    x.style().opacity,
    undefined,
    "opacity belongs to content, not slab or sidebar",
  );
  x.harness.unmount();
});

test("cancelled and unmounted fades cannot dispatch stale roots; reduced motion uses no fade", () => {
  const x = setup();
  let swaps = 0;
  x.contentTransition.current.fadeOut(() => swaps++);
  const stale = x.timings.at(-1)!;
  x.contentTransition.current.reveal();
  stale.complete(true);
  assert.equal(swaps, 0);
  x.contentTransition.current.fadeOut(() => swaps++);
  const removed = x.timings.at(-1)!;
  x.harness.unmount();
  removed.complete(true);
  assert.equal(swaps, 0);
  const reduced = setup(true);
  reduced.contentTransition.current.fadeOut(() => swaps++);
  assert.equal(
    reduced.opacity(),
    1,
    "reduced motion never hides content while waiting for paint",
  );
  reduced.contentTransition.current.reveal();
  assert.equal(swaps, 1);
  assert.equal(reduced.timings.length, 0);
  reduced.harness.unmount();
});

test("closing during a pending fade restores opacity and the settled content's AX and pointer access", () => {
  for (const close of ["sliver", "controlled"]) {
    const x = setup();
    let swaps = 0;
    x.set({ visible: true, presented: true });
    x.lastComplete();
    x.contentTransition.current.fadeOut(() => swaps++);
    const staleFade = x.timings.at(-1)!;
    assert.equal(x.opacity(), 0);
    if (close === "sliver") {
      x.find("workspace-sidebar-close-sliver").props.onPress();
      x.render();
    } else x.set({ visible: false });
    assert.equal(x.opacity(), 1);
    staleFade.complete(true);
    assert.equal(swaps, 0, "cancelled fade cannot swap roots after close");
    x.lastComplete();
    assert.equal(x.offset(), 0);
    assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "none");
    assert.equal(
      x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
      true,
    );
    assert.equal(
      x.find("workspace-slab-content").props.accessibilityElementsHidden,
      false,
    );
    assert.equal(x.harness.value().props.pointerEvents, "auto");
    assert.equal(
      x
        .all()
        .some((node) => node.props.testID === "workspace-sidebar-close-sliver"),
      false,
    );
    x.harness.unmount();
  }
});

test("an interrupted fade restores visibility and stale interruption cannot cancel a newer fade", () => {
  const x = setup();
  let swaps = 0;
  x.set({ visible: true, presented: true });
  x.lastComplete();
  x.contentTransition.current.fadeOut(() => swaps++);
  const first = x.timings.at(-1)!;
  first.complete(false);
  assert.equal(x.opacity(), 1);
  assert.equal(swaps, 0);
  x.contentTransition.current.fadeOut(() => swaps++);
  const second = x.timings.at(-1)!;
  first.complete(false);
  assert.equal(x.opacity(), 0);
  second.complete(true);
  assert.equal(swaps, 1);
  x.contentTransition.current.reveal();
  assert.equal(x.opacity(), 1);
  x.harness.unmount();
});

// Reproduced on device: a cancelled edge gesture left presented=true while the
// requested and painted endpoint were closed. Completion must reconcile all three.
test("a zero-distance/cancelled endpoint cannot leave invisible sidebar AX exposed", () => {
  const x = setup();
  x.set({ presented: true, visible: false });
  x.reconcile();
  assert.equal(x.offset(), 0);
  assert.equal(x.counts().dismissed, 1);
  assert.equal(
    x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
    true,
  );
  const content = x.find("workspace-slab-content");
  assert.equal(content.props.accessibilityElementsHidden, false);
  x.harness.unmount();
});
test("UI endpoint reconciliation survives a missed completion and rejects an old endpoint after reopening", () => {
  const x = setup();
  x.set({ visible: true, presented: true });
  x.lastComplete();
  x.set({ visible: false });
  x.lastComplete(false);
  x.reconcile();
  assert.equal(x.counts().dismissed, 1);
  assert.equal(
    x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
    true,
  );
  x.set({ visible: true, presented: true });
  x.shared[0]!.value = 0;
  x.reconcile();
  assert.equal(
    x.counts().dismissed,
    1,
    "stale closed acknowledgement cannot dismiss a new open request",
  );
  x.shared[0]!.value = 310;
  x.lastComplete();
  assert.equal(
    x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
    false,
  );
  x.harness.unmount();
});

test("desktop rotation and portrait return retarget the sidebar without losing its content or settled access", () => {
  const x = setup();
  x.set({ visible: true, presented: true });
  x.lastComplete();
  x.set({ obscured: true });
  x.rotate(874, 402);
  assert.equal(
    x.find("workspace-sidebar-base").props.style[1].width,
    sidebarPushWidth(874),
  );
  assert.equal(x.style().transform[0].translateX, sidebarPushWidth(874));
  x.lastComplete();
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "none");
  x.rotate(402, 874);
  x.lastComplete();
  x.set({ obscured: false });
  assert.equal(x.style().transform[0].translateX, 310);
  assert.equal(
    x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
    false,
  );
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  assert.ok(x.all().includes(x.child));
  x.set({ visible: false });
  x.lastComplete();
  assert.equal(x.style().transform[0].translateX, 0);
  assert.equal(x.opacity(), 1);
  assert.equal(
    x.find("workspace-slab-content").props.accessibilityElementsHidden,
    false,
  );
  assert.equal(x.counts().dismissed, 1);
  x.harness.unmount();
});

test("a wide window exposes a bounded sidebar beside the same fully accessible navigator", () => {
  const x = setup(false, 1024, 768);
  assert.equal(x.find("workspace-sidebar-base").props.style[1].width, 300);
  assert.equal(x.find("workspace-content-slab").props.style[2].read().marginLeft, 300,);
  assert.equal(x.style().transform[0].translateX, 0);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  assert.equal(
    x.find("workspace-sidebar-base").props.accessibilityElementsHidden,
    false,
  );
  assert.equal(
    x.find("workspace-slab-content").props.accessibilityElementsHidden,
    false,
  );
  assert.equal(
    x.find("workspace-slab-content").props.importantForAccessibility,
    "auto",
  );
  assert.ok(x.all().includes(x.child));
  assert.ok(x.all().includes(x.sidebar));
  assert.equal(
    x.all().some((node) => node.type === "GestureDetector"),
    false,
  );
  assert.equal(x.hardwareBack(), undefined);
  assert.equal(
    x.focus.some((event) => event[0] === "active" && event[2]),
    false,
  );
  x.harness.value().props.onAccessibilityEscape();
  assert.deepEqual(x.changes, []);
  assert.equal(x.timings.length, 0);
  x.harness.unmount();
});

test("resize out of an opening drawer fences old completion and keeps the navigator and sidebar mounted", () => {
  const x = setup();
  x.set({ visible: true, presented: true });
  const opening = x.timings.at(-1)!;
  x.rotate(1194, 834);
  opening.complete(true);
  x.render();
  assert.equal(x.offset(), 0);
  assert.equal(x.find("workspace-content-slab").props.style[2].read().marginLeft, 300,);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  assert.equal(
    x.find("workspace-slab-content").props.accessibilityElementsHidden,
    false,
  );
  assert.equal(x.hardwareBack(), undefined);
  assert.equal(
    x.focus.some((event) => event[0] === "focus"),
    false,
  );
  assert.ok(x.all().includes(x.child));
  assert.ok(x.all().includes(x.sidebar));

  // App resets the compact request when crossing the breakpoint. The visible
  // rail is independent of that modal request, then Split View returns closed.
  x.set({ visible: false, presented: false });
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  x.rotate(744, 1133);
  assert.equal(x.find("workspace-content-slab").props.style[2].read().marginLeft, 0,);
  assert.equal(x.find("workspace-sidebar-base").props.style[1].width, 360);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "none");
  assert.equal(x.style().transform[0].translateX, 0);
  assert.equal(
    x.find("workspace-slab-content").props.accessibilityElementsHidden,
    false,
  );
  assert.ok(x.all().includes(x.child));
  assert.ok(x.all().includes(x.sidebar));
  x.harness.unmount();
});

test("Settings suspends both wide columns and returns without changing layout or taking modal focus", () => {
  const x = setup(false, 1366, 1024);
  x.set({ obscured: true });
  assert.equal(x.harness.value().props.pointerEvents, "none");
  assert.equal(x.harness.value().props.accessibilityElementsHidden, true);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "none");
  assert.equal(x.hardwareBack(), undefined);
  x.rotate(1024, 1366);
  x.set({ obscured: false });
  assert.equal(x.harness.value().props.accessibilityElementsHidden, false);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  assert.equal(
    x.find("workspace-slab-content").props.accessibilityElementsHidden,
    false,
  );
  assert.equal(x.find("workspace-content-slab").props.style[2].read().marginLeft, 300,);
  assert.equal(
    x.focus.some((event) => event[0] === "focus"),
    false,
  );
  assert.ok(x.all().includes(x.child));
  x.harness.unmount();
});

test("wide nested guarded Back uses only the content edge and never closes the sidebar", () => {
  const x = setup(false, 1024, 768);
  let backs = 0;
  x.set({ onEdgeBack: () => backs++ });
  const edge = x.find("workspace-sidebar-reveal-edge");
  assert.equal(edge.props.accessible, false);
  assert.equal(edge.props.focusable, false);
  assert.equal(edge.props.importantForAccessibility, "no-hide-descendants");
  assert.equal(edge.props.style.width, 24);
  x.handlers().onStart();
  x.handlers().onUpdate({ translationX: 160 });
  x.handlers().onEnd({ translationX: 160, velocityX: 0 });
  assert.equal(backs, 1);
  assert.equal(x.style().transform[0].translateX, 0);
  assert.equal(x.counts().reveals, 0);
  assert.deepEqual(x.changes, []);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  x.set({ gestureEnabled: false });
  assert.equal(
    x.all().some((node) => node.type === "GestureDetector"),
    false,
  );
  x.harness.unmount();
});


test("a critical write blocks revealing but never swipe, sliver, Android Back or accessibility closing", () => {
  for (const path of ["swipe", "sliver", "hardware", "escape"] as const) {
    const env = setup(true);
    env.set({ visible: true, presented: true });
    env.reconcile();
    env.set({ gestureEnabled: false });
    assert.equal(
      env.settings.enabled,
      true,
      "the presented close gesture stays enabled",
    );
    if (path === "swipe") {
      env.handlers().onStart();
      env.handlers().onUpdate({ translationX: -500 });
      env.handlers().onEnd({ translationX: -500, velocityX: -500 });
    } else if (path === "sliver")
      env.find("workspace-sidebar-close-sliver").props.onPress();
    else if (path === "hardware") assert.equal(env.hardwareBack(), true);
    else env.all()[0]!.props.onAccessibilityEscape();
    env.render();
    assert.equal(env.changes.at(-1), false, path);
    env.reconcile();
    assert.equal(
      env.settings.enabled,
      false,
      "closed drawer cannot be revealed while blocked",
    );
    env.harness.unmount();
  }
});

// Folded Duo portrait/landscape, unfolded Duo (beside the persistent
// sidebar), ordinary iPhone portrait/landscape, iPads.
const RAIL_WINDOWS = [
  { name: "folded Duo", width: 466, height: 678, right: 83, rail: true },
  { name: "folded Duo landscape", width: 678, height: 466, right: 104, rail: true, },
  { name: "unfolded Duo", width: 951, height: 669, right: 104, rail: true },
  { name: "iPhone portrait", width: 402, height: 874, right: 0, rail: false },
  { name: "iPhone landscape", width: 874, height: 402, right: 62, rail: false },
  { name: "iPad landscape", width: 1194, height: 834, right: 0, rail: false },
  { name: "iPad 13in", width: 1366, height: 1024, right: 0, rail: false },
] as const;

function railSetup(reduced = false) {
  const x = setup(reduced, 466, 678);
  const rails: { fullScreen: boolean; onToggle: () => void; header: any }[] = [];
  x.set({
    quickRail: (fullScreen: boolean, onToggle: () => void, header: unknown) => {
      rails.push({ fullScreen, onToggle, header });
      return { type: "QuickRail", props: { fullScreen, onToggle } };
    },
  });
  // Inside out: the column-header registry wraps the navigator, then the
  // floating-inset provider, then the rail-motion provider (inset, full
  // screen, progress), then the rail-available provider the
  // headers read.
  const parentOf = (child: unknown) =>
    x.all().find((node) => node.props.children === child)!;
  const columnProvider = () => parentOf(x.child);
  const insetProvider = () => parentOf(columnProvider());
  const motionProvider = () => parentOf(insetProvider());
  const provider = () => parentOf(motionProvider());
  return { ...x, rails, provider, motionProvider, insetProvider, columnProvider, };
}

test("the quick rail exists on every corner-sensor window (folded and unfolded Duo), never on ordinary iPhones or iPads", () => {
  for (const window of RAIL_WINDOWS) {
    const x = railSetup();
    x.window(window.width, window.height, { right: window.right, top: 0 });
    const rail = x.all().find((node) => node.props.testID === "workspace-quick-rail");
    assert.equal(Boolean(rail), window.rail, window.name);
    if (rail) {
      // One host: the toggle capsule and the grouped capsule; no separate
      // pull tab.
      assert.equal(rail.props.pointerEvents, "box-none");
      assert.equal(x.rails.at(-1)!.fullScreen, false);
      assert.equal(
        x.all().some((n) => n.props.testID === "workspace-quick-rail-tab-host"),
        false,
      );
      // The host never fades (glass stops rendering under a translucent
      // ancestor); the capsules fade themselves.
      assert.equal(
        [rail.props.style].flat().some((style: any) => style?.read || style?.opacity !== undefined),
        false,
      );
    }
    assert.equal(x.provider().props.value, window.rail, `${window.name} context`,
    );
    // Rail mode: content keeps clear of the rail column where it exists.
    assert.equal(
      x.insetProvider().props.value,
      window.rail ? 66 : 0,
      window.name,
    );
    const motion = x.motionProvider().props.value;
    if (window.rail) {
      assert.equal(motion.inset, 66);
      assert.equal(motion.fullScreen, false);
      assert.deepEqual(Object.keys(motion).sort(), [
        "fullScreen",
        "inset",
        "progress",
      ]);
      assert.equal(motion.progress.value, 0);
    } else assert.equal(motion, null, window.name);
    // The status column never gets a backdrop: only the capsules show.
    assert.equal(
      x.all().some((n) => n.props.testID === "workspace-status-column-fade"),
      false,
    );
    x.harness.unmount();
  }
  const signedOut = railSetup();
  signedOut.window(466, 678, { right: 83 });
  signedOut.set({ enabled: false });
  assert.equal(
    signedOut.all().some((n) => n.props.testID === "workspace-quick-rail"),
    false,
  );
  signedOut.harness.unmount();
  const omitted = setup(false, 466, 678);
  omitted.window(466, 678, { right: 83 });
  assert.equal(
    omitted.all().some((n) => n.props.testID === "workspace-quick-rail"),
    false,
  );
  omitted.harness.unmount();
});

test("the rail's column clears the expanded camera status cluster, bounded by the composer's bottom edge, after the page content", () => {
  const x = railSetup();
  x.window(466, 678, { right: 83, bottom: 34 });
  const idle = Object.assign({}, ...[x.find("workspace-quick-rail").props.style].flat());
  assert.equal(idle.top, 160, "idle keeps the original position");
  x.status(36);
  const host = x.find("workspace-quick-rail");
  const style = Object.assign({}, ...[host.props.style].flat());
  assert.equal(style.top, 196);
  // 56pt wide, centered 48pt from the right edge: the clock/Wi‑Fi column
  // at 418pt.
  assert.equal(style.width, 56);
  assert.equal(style.right, 20);
  assert.ok(Math.abs(466 - style.right - style.width / 2 - 418) <= 0.5);
  // Bounded by the composer's bottom edge (safe area plus 4pt page padding);
  // its sections hug the top and the
  // empty rest passes touches through (box-none).
  assert.equal(style.bottom, 38);
  assert.equal(678 - style.bottom, 640);
  assert.equal(host.props.pointerEvents, "box-none");
  const order = x.all();
  assert.ok(
    order.indexOf(x.child) < order.indexOf(host),
    "page content reads first",
  );
  // Outside the sliding slab: the drawer's slide never moves the rail, and
  // a touch on it still blurs a focused composer.
  assert.equal(
    elements(x.find("workspace-content-slab") as Element).includes(host),
    false,
  );
  assert.equal(typeof host.props.onTouchStart, "function");
  assert.equal(typeof host.props.onTouchEnd, "function");
  x.status(0);
  assert.equal(Object.assign({}, ...[x.find("workspace-quick-rail").props.style].flat()).top, 160);
  x.harness.unmount();
});

test("Full screen gives the rail column to the content in one 250ms UI-thread transition; the toggle and the header's controls stay", () => {
  for (const reduced of [false, true]) {
    const x = railSetup(reduced);
    x.window(466, 678, { right: 83 });
    const frame = () =>
      Object.assign({}, ...[x.find("workspace-quick-rail").props.style].flat());
    const before = { frame: frame(), timings: x.timings.length };
    x.rails.at(-1)!.onToggle();
    x.render();
    assert.deepEqual(x.railWrites, [true], "saved per device");
    assert.equal(x.rails.at(-1)!.fullScreen, true);
    const motion = x.motionProvider().props.value;
    assert.equal(motion.fullScreen, true);
    if (reduced) {
      assert.equal(
        x.timings.length,
        before.timings,
        "Reduce Motion swaps at once",
      );
      assert.equal(motion.progress.value, 1);
    } else {
      // One 250ms ease-out on the slab's curve drives the content's inset,
      // and the grouped capsule's fade together.
      assert.equal(x.timings.length, before.timings + 1);
      const timing = x.timings.at(-1)!;
      assert.equal(timing.target, 1);
      assert.equal(timing.config.duration, 250);
      assert.deepEqual([...timing.config.easing], [0.2, 0.8, 0.2, 1]);
    }
    // Content runs to the 12pt margin; the header keeps the same controls;
    // the toggle (now restore) stays at the shared spot over the content
    // with no backdrop behind the column: only the capsules show.
    assert.equal(x.insetProvider().props.value, 0);
    assert.equal(x.provider().props.value, true);
    assert.deepEqual(frame(), before.frame);
    assert.equal(frame().top, 160);
    assert.equal(
      x.find("workspace-quick-rail").props.pointerEvents,
      "box-none",
    );
    assert.equal(
      x.all().some((n) => n.props.testID === "workspace-status-column-fade"),
      false,
    );
    // The same toggle returns the rail column to the rail.
    x.rails.at(-1)!.onToggle();
    x.render();
    assert.deepEqual(x.railWrites, [true, false]);
    assert.equal(x.rails.at(-1)!.fullScreen, false);
    assert.equal(x.insetProvider().props.value, 66);
    assert.equal(x.motionProvider().props.value.fullScreen, false);
    if (!reduced) assert.equal(x.timings.at(-1)!.target, 0);
    x.harness.unmount();
  }
});

test("unfolded Duo: the same rail sits on the status column under its measured cluster beside the persistent sidebar, open or closed, with the same full-screen semantics", () => {
  for (const reduced of [false, true]) {
    const x = railSetup(reduced);
    x.window(951, 669, { right: 104, bottom: 20 });
    x.set({ sidebarCollapsed: true, sidebarRestored: true });
    const host = () => x.find("workspace-quick-rail");
    const frame = () => Object.assign({}, ...[host().props.style].flat());
    // Idle: 8pt under the cluster, centered on the status column 48pt from
    // the right edge, bounded by the composer's baseline.
    assert.equal(frame().top, 108);
    assert.equal(frame().right, 20);
    assert.equal(frame().width, 56);
    assert.equal(frame().bottom, 24);
    assert.ok(Math.abs(951 - frame().right - frame().width / 2 - (951 - 48)) <= 0.5);
    assert.equal(host().props.pointerEvents, "box-none");
    const order = x.all();
    assert.ok(
      order.indexOf(x.child) < order.indexOf(host()),
      "page content reads first",
    );
    // Headers read the rail, so they keep only More (same as folded).
    assert.equal(x.provider().props.value, true);
    const slab = () => x.find("workspace-content-slab").props.style[2].read();
    const sidebar = () => x.find("workspace-sidebar-base");
    // The rail lives in the slab, anchored to the window's right edge: the
    // sidebar opens at the left edge and only moves the slab's left edge, so
    // the rail, its frame and the content's rail inset never change.
    for (const collapsed of [true, false, true]) {
      x.set({ sidebarCollapsed: collapsed });
      assert.equal(slab().marginLeft, collapsed ? 0 : 300);
      assert.equal(sidebar().props.pointerEvents, collapsed ? "none" : "auto");
      assert.deepEqual(frame(), { ...frame(), top: 108, right: 20, width: 56 });
      assert.equal(
        x.insetProvider().props.value,
        66,
        "content keeps clear of the rail",
      );
    }
    // Full screen: the same toggle, the group hidden, content to the margin,
    // no backdrop behind the 108–152pt idle restore capsule.
    x.rails.at(-1)!.onToggle();
    x.render();
    assert.deepEqual(x.railWrites, [true], "the same saved per-device choice");
    assert.equal(x.rails.at(-1)!.fullScreen, true);
    assert.equal(x.insetProvider().props.value, 0);
    assert.equal(
      x.provider().props.value,
      true,
      "the header keeps the same controls",
    );
    assert.equal(frame().top, 108, "the toggle stays at its spot");
    if (!reduced) assert.equal(x.timings.at(-1)!.config.duration, 250);
    assert.equal(
      x.all().some((n) => n.props.testID === "workspace-status-column-fade"),
      false,
    );
    x.set({ sidebarCollapsed: false });
    assert.equal(
      x.insetProvider().props.value,
      0,
      "full screen survives the sidebar",
    );
    x.rails.at(-1)!.onToggle();
    x.render();
    assert.equal(x.insetProvider().props.value, 66);
    x.harness.unmount();
  }
});

test("a focused chat's bar controls reach the rail in both modes (the column hides them in full screen, where the chat's own bar returns)", () => {
  for (const [name, width, height, right, toggleBottom, headerBottom] of [
    ["folded", 466, 678, 83, 204, 356],
    ["unfolded", 951, 669, 104, 152, 304],
  ] as const) {
    const x = railSetup();
    x.window(width, height, { right });
    const register = x.columnProvider().props.value;
    const chat = Symbol("chat");
    const controls = { leading: {}, identity: {}, more: {} };
    // No chat focused (another page): no header controls.
    assert.equal(x.rails.at(-1)!.header, null);
    assert.ok(headerBottom > toggleBottom);
    register(chat, controls);
    x.render();
    assert.equal(x.rails.at(-1)!.header, controls, name);
    // Full screen: the rail still receives them (it fades its column
    // capsule out; the chat's bar shows them) and the shell renders no
    // extra top header of its own.
    x.rails.at(-1)!.onToggle();
    x.render();
    assert.equal(x.rails.at(-1)!.fullScreen, true);
    assert.equal(x.rails.at(-1)!.header, controls);
    assert.equal(x.motionProvider().props.value.fullScreen, true, name);
    // Another owner's withdrawal never clears this chat's controls; its own
    // does (blur, unmount).
    register(Symbol("other"), null);
    x.render();
    assert.equal(x.rails.at(-1)!.header, controls);
    register(chat, null);
    x.render();
    assert.equal(x.rails.at(-1)!.header, null);
    x.harness.unmount();
  }
  // Ordinary iPhones have no rail: nothing reaches a rail or the clearance.
  const phone = railSetup();
  phone.window(402, 874, { right: 0 });
  phone
    .columnProvider()
    .props.value(Symbol("chat"), { leading: {}, identity: {}, more: {} });
  phone.render();
  assert.equal(phone.motionProvider().props.value, null);
  assert.equal(
    phone.all().some((n) => n.props.testID === "workspace-quick-rail"),
    false,
  );
  phone.harness.unmount();
});

test("Duo wide windows hide and show the persistent sidebar with one UI-thread animation; restoring the saved choice does not animate", () => {
  for (const reduced of [false, true]) {
    const x = setup(reduced, 1194, 834);
    x.window(1194, 834, { right: 83 });
    const slab = () => x.find("workspace-content-slab").props.style[2].read();
    const sidebar = () => x.find("workspace-sidebar-base");
    // The saved choice arrives with the restore: applied at once.
    const before = x.timings.length;
    x.set({ sidebarCollapsed: true, sidebarRestored: true });
    assert.equal(x.timings.length, before, "restoring never animates");
    assert.equal(slab().marginLeft, 0, "content takes the full width");
    assert.equal(sidebar().props.pointerEvents, "none");
    assert.equal(sidebar().props.accessibilityElementsHidden, true);
    // A person's toggle animates the slab's edge with the same curve.
    x.set({ sidebarCollapsed: false });
    if (reduced) assert.equal(x.timings.length, before);
    else {
      const timing = x.timings.at(-1)!;
      assert.equal(timing.target, 1);
      assert.equal(timing.config.duration, 250);
    }
    assert.equal(slab().marginLeft, 300);
    assert.equal(sidebar().props.pointerEvents, "auto");
    x.harness.unmount();
  }
  // Narrow windows ignore it: the drawer is unaffected.
  const narrow = setup(false, 390, 844);
  narrow.set({ sidebarCollapsed: true, sidebarRestored: true });
  assert.equal(
    narrow.find("workspace-content-slab").props.style[2].read().marginLeft,
    0,
  );
  narrow.harness.unmount();
});

test("a tap anywhere in the slab blurs a focused chat composer without taking the tap", async () => {
  const { chatComposerFocus } = await import(
    "../../features/assistant/chatComposerFocus"
  );
  const x = railSetup();
  x.window(466, 678, { right: 83 });
  const body = x.find("workspace-slab-body");
  // It only observes: no responder or press props that could claim a touch
  // from the link, button, message action or rail control under the finger.
  for (const claim of [
    "onStartShouldSetResponder",
    "onStartShouldSetResponderCapture",
    "onMoveShouldSetResponder",
    "onResponderGrant",
    "onPress",
  ])
    assert.equal(body.props[claim], undefined, claim);
  let blurs = 0;
  const input = { blur: () => blurs++ };
  chatComposerFocus.focused(input);
  // Any touch in the slab (transcript, header, rail): the touched control
  // keeps the responder and gets its release; the composer blurs as that
  // same touch ends, after the press is decided.
  body.props.onTouchStart();
  assert.equal(blurs, 0, "nothing moves under the finger while it is down");
  body.props.onTouchEnd();
  assert.equal(blurs, 1);
  // A touch that began in the composer (send, attach, the text itself) keeps it.
  chatComposerFocus.touchStartInComposer();
  body.props.onTouchStart();
  body.props.onTouchEnd();
  assert.equal(blurs, 1);
  chatComposerFocus.blurred(input);
  x.harness.unmount();
});

test("P9 ordinary wide windows retain the visible base sidebar even with a saved Duo choice", () => {
  const x = setup(false, 1194, 834);
  try {
    x.set({ sidebarCollapsed: true, sidebarRestored: true });
    assert.equal(
      x.find("workspace-content-slab").props.style[2].read().marginLeft,
      300,
    );
    assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
    assert.equal(x.find("workspace-slab-body").props.onTouchEnd, undefined);
  } finally {
    x.harness.unmount();
  }
});
