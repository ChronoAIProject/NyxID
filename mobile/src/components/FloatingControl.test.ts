import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";
import { darkColors, lightColors } from "../theme/mobileTheme";

type E = { type: unknown; props: Record<string, any> };
const flat = (style: unknown) =>
  Object.assign({}, ...[style].flat(Infinity).filter(Boolean));

function load(liquidGlass: boolean, mode: "light" | "dark" = "light") {
  const colors = mode === "light" ? lightColors : darkColors;
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      StyleSheet: { create: (v: unknown) => v, hairlineWidth: 0.5 },
    },
    "expo-glass-effect": {
      GlassView: "GlassView",
      GlassContainer: "GlassContainer",
      isLiquidGlassAvailable: () => liquidGlass,
    },
    "../theme/ThemeContext": { useTheme: () => ({ mode, colors }) },
  });
  return {
    h,
    colors,
    module: h.load(
      fileURLToPath(new URL("./FloatingControl.tsx", import.meta.url)),
    ) as any,
  };
}

test("on iOS 26 a floating capsule is liquid glass with the theme's tint, style 'regular', interactive, in the app's colour scheme", () => {
  for (const [mode, tint] of [
    ["light", "#F9F9F999"],
    ["dark", "#35353999"],
  ] as const) {
    const { h, module } = load(true, mode);
    h.mount(() =>
      module.FloatingCapsule({
        testID: "group",
        style: { width: 44 },
        children: ["New chat", "Scan QR"],
      }),
    );
    const glass = h.value();
    assert.equal(glass.type, "GlassView");
    assert.equal(glass.props.testID, "group");
    // The same explicit tint for every control: the header-button token at
    // 60% alpha, never a system-chosen tint.
    assert.equal(glass.props.tintColor, tint);
    assert.equal(glass.props.colorScheme, mode, "the app's theme, not the system's");
    assert.equal(glass.props.isInteractive, true);
    assert.equal(
      JSON.stringify(glass.props.glassEffectStyle),
      JSON.stringify({ style: "regular", animate: true, animationDuration: 0.25 }),
    );
    // Glass only: no opaque fill, border or shadow on top of it.
    const style = flat(glass.props.style);
    assert.deepEqual(Object.keys(style).sort(), ["borderRadius", "width"]);
    assert.equal(style.borderRadius, 22);
    assert.equal(JSON.stringify(glass.props.children), '["New chat","Scan QR"]');
    // Hiding dissolves the glass natively (never an ancestor's opacity).
    h.mount(() => module.FloatingCapsule({ visible: false, children: null }));
    assert.equal(h.value().props.glassEffectStyle.style, "none");
    // ...and clears its tint, which would otherwise still draw.
    assert.match(h.value().props.tintColor, /^#[0-9A-F]{6}00$/i);
    h.mount(() => module.FloatingCapsule({ visible: false, glass: "railOpaque", children: null }));
    assert.match(h.value().props.tintColor, /^#[0-9A-F]{6}00$/i);
    h.unmount();
  }
});

test("without liquid glass a floating capsule is the theme's opaque surface with the same token", () => {
  for (const mode of ["light", "dark"] as const) {
    const { h, module, colors } = load(false, mode);
    h.mount(() => module.FloatingCapsule({ testID: "toggle", children: "Icon" }));
    const capsule = h.value();
    assert.equal(capsule.type, "View");
    const style = flat(capsule.props.style);
    assert.equal(style.backgroundColor, colors.floatingControlBg);
    assert.equal(style.borderColor, colors.floatingControlBorder);
    assert.equal(style.borderWidth, 0.5);
    assert.equal(style.borderRadius, 22);
    assert.equal(style.shadowRadius, mode === "light" ? 10 : undefined);
    h.unmount();
  }
});

test("capsules that belong together share one native glass container on iOS 26", () => {
  for (const [liquidGlass, type] of [
    [true, "GlassContainer"],
    [false, "View"],
  ] as const) {
    const { h, module } = load(liquidGlass);
    assert.equal(module.floatingGlassAvailable(), liquidGlass);
    h.mount(() => module.FloatingGroup({ spacing: 4, testID: "rail", children: "Capsules" }));
    assert.equal(h.value().type, type);
    assert.equal(h.value().props.testID, "rail");
    if (liquidGlass) assert.equal(h.value().props.spacing, 4);
    h.unmount();
  }
});

test("a floating button is one control in its own 44pt capsule; items highlight in the capsule's shape and dim when disabled", () => {
  const { h, module, colors } = load(true);
  let presses = 0;
  h.mount(() =>
    module.FloatingButton({
      accessibilityLabel: "Chat options",
      onPress: () => presses++,
      capsuleStyle: { alignSelf: "flex-start" },
      children: "Icon",
    }),
  );
  const capsule = h.value();
  assert.equal(capsule.type, module.FloatingCapsule);
  const capsuleStyle = flat(capsule.props.style);
  assert.equal(capsuleStyle.minWidth, 44);
  assert.equal(capsuleStyle.minHeight, 44);
  assert.equal(capsuleStyle.alignSelf, "flex-start");
  const item = capsule.props.children;
  assert.equal(item.type, module.FloatingItem);
  assert.equal(item.props.accessibilityLabel, "Chat options");
  item.props.onPress();
  assert.equal(presses, 1);
  h.unmount();
  for (const disabled of [false, true]) {
    h.mount(() =>
      module.FloatingItem({
        accessibilityLabel: "New chat",
        onPress() {},
        disabled,
        style: { paddingLeft: 6 },
        children: "Icon",
      }),
    );
    const pressable = h.value();
    assert.equal(pressable.type, "Pressable");
    assert.equal(pressable.props.accessibilityRole, "button");
    assert.equal(pressable.props.accessibilityState.disabled, disabled);
    const rest = flat(pressable.props.style({ pressed: false }));
    assert.equal(rest.minWidth, 44);
    assert.equal(rest.minHeight, 44);
    assert.equal(rest.borderRadius, 22, "a highlight in the capsule's shape");
    assert.equal(rest.paddingLeft, 6, "the control's own layout");
    assert.equal(rest.backgroundColor, undefined, "no surface of its own");
    assert.equal(rest.opacity, disabled ? 0.4 : undefined);
    const pressed = flat(pressable.props.style({ pressed: true }));
    assert.equal(pressed.backgroundColor, disabled ? undefined : colors.segmentTrack);
    h.unmount();
  }
});
