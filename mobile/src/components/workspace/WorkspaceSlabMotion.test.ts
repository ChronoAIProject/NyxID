import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";

type E = { type: unknown; props: Record<string, any> };
const flat = (style: unknown) =>
  Object.assign({}, ...[style].flat(Infinity).filter(Boolean));

function load(
  rail: {
    inset: number;
    fullScreen: boolean;
    progress: { value: number };
  } | null,
) {
  const animation = {
    duration: () => animation,
    reduceMotion: () => animation,
    withInitialValues: () => animation,
  };
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native-reanimated": {
      __esModule: true,
      default: { View: "Animated.View" },
      FadeIn: animation,
      FadeOut: animation,
      ZoomIn: animation,
      ReduceMotion: { System: "system" },
      // The worklet runs on the UI thread; here it is evaluated directly.
      useAnimatedStyle: (worklet: () => object) => ({ animated: worklet() }),
    },
    "./workspaceFloatingInset": {
      WorkspaceRailMotionContext: { value: rail },
    },
  });
  const motion = h.load(
    fileURLToPath(new URL("./WorkspaceSlabMotion.tsx", import.meta.url)),
  ) as any;
  // Render through the harness so the hooks run in a component.
  const render = (component: string, props: object) => {
    h.mount(() => motion[component](props));
    const value = h.value();
    h.unmount();
    return value;
  };
  return { render, module: motion };
}

test("the content inset animates from the rail's 66pt to the 12pt-margin layout on the UI thread", () => {
  for (const [progress, expected] of [
    [0, 20 + 66], // rail mode: own 20pt + the rail inset
    [0.5, 20 + 33], // mid-transition: the content reflows smoothly
    [1, 20], // full screen: only its own padding (the 12pt-margin layout)
  ] as const) {
    const motion = load({
      inset: 66,
      fullScreen: progress === 1,
      progress: { value: progress },
    });
    const view = motion.render("RailInsetView", {
      ownPaddingRight: 20,
      style: { padding: 20 },
      children: "Rows",
    });
    assert.equal(view.type, "Animated.View");
    const style = [view.props.style].flat();
    assert.equal(
      style[1].animated.paddingRight,
      expected,
      `progress ${progress}`,
    );
    assert.equal(flat(style[0]).padding, 20);
  }
  // No rail (ordinary phones, iPads): a plain static view.
  const plain = load(null).render("RailInsetView", {
    style: { padding: 20 },
    children: "Rows",
  });
  assert.deepEqual(flat(plain.props.style), { padding: 20 });
});

test("the grouped capsule fades out with full screen and back in with rail mode", () => {
  for (const [progress, opacity] of [
    [0, 1],
    [0.4, 0.6],
    [1, 0],
  ] as const) {
    const motion = load({
      inset: 66,
      fullScreen: progress === 1,
      progress: { value: progress },
    });
    const view = motion.render("RailShortcutsView", { children: "Capsule" });
    assert.ok(
      Math.abs([view.props.style].flat()[1].animated.opacity - opacity) < 1e-9,
    );
  }
});

test("over liquid glass the grouped capsule's wrapper never fades (glass under a fading ancestor stops rendering)", () => {
  const motion = load({ inset: 66, fullScreen: true, progress: { value: 1 } });
  const view = motion.render("RailShortcutsView", {
    fadeOpacity: false,
    style: { gap: 4 },
    children: "Capsule",
  });
  assert.deepEqual(flat(view.props.style), { gap: 4 });
});

test("the shortcuts' icons fade with the grouped capsule on the UI thread (the only controls that change between modes)", () => {
  for (const [progress, opacity] of [
    [0, 1],
    [0.25, 0.75],
    [1, 0],
  ] as const) {
    const motion = load({
      inset: 66,
      fullScreen: progress === 1,
      progress: { value: progress },
    });
    const icon = motion.render("RailShortcutIcon", { children: "New chat" });
    assert.equal(icon.type, "Animated.View");
    assert.ok(
      Math.abs([icon.props.style].flat()[1].animated.opacity - opacity) < 1e-9,
    );
    assert.equal(icon.props.children, "New chat");
  }
  assert.deepEqual(
    Object.keys(load(null).module).sort(),
    [
      "HEADER_ACTION_ENTERING",
      "HEADER_ACTION_EXITING",
      "MotionView",
      "RailInsetView",
      "RailShortcutIcon",
      "RailShortcutsView",
    ],
    "capsule menus own their size animation while the rail header stays in place",
  );
});
