import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import {
  clampSidebarOffset,
  settleSidebarOpen,
  sidebarPushWidth,
  SIDEBAR_EDGE_WIDTH,
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
function setup(reduced = false) {
  let dimensions = { width: 402, height: 874 };
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
      View: "View",
      Pressable: "Pressable",
      useWindowDimensions: () => dimensions,
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
    rotate: (width: number, height: number) => {
      dimensions = { width, height };
      render();
    },
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
  assert.ok(x.all().includes(x.child));
  x.harness.unmount();
});

test("edge pan follows the finger, clamps travel and settles at the projected threshold", () => {
  const x = setup();
  assert.equal(x.settings.activeOffsetX, 18);
  assert.deepEqual([...x.settings.failOffsetY], [-10, 10]);
  assert.equal(x.find("workspace-sidebar-reveal-edge").props.style.width, 24);
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
  assert.equal(x.find("workspace-sidebar-base").props.style[1].width, sidebarPushWidth(874));
  assert.equal(x.style().transform[0].translateX, sidebarPushWidth(874));
  x.lastComplete();
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "none");
  x.rotate(402, 874);
  x.lastComplete();
  x.set({ obscured: false });
  assert.equal(x.style().transform[0].translateX, 310);
  assert.equal(x.find("workspace-sidebar-base").props.accessibilityElementsHidden, false);
  assert.equal(x.find("workspace-sidebar-base").props.pointerEvents, "auto");
  assert.ok(x.all().includes(x.child));
  x.set({ visible: false });
  x.lastComplete();
  assert.equal(x.style().transform[0].translateX, 0);
  assert.equal(x.opacity(), 1);
  assert.equal(x.find("workspace-slab-content").props.accessibilityElementsHidden, false);
  assert.equal(x.counts().dismissed, 1);
});
