import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any>; key?: number };
function setup() {
  let fontScale = 1;
  const jsx = (type: unknown, props: Element["props"], key?: number) => ({
    type,
    props,
    key,
  });
  const dynamicText = {};
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Text: "Text",
      Pressable: "Pressable",
      View: "View",
      useWindowDimensions: () => ({ fontScale }),
      StyleSheet: { create: (value: unknown) => value },
    },
    "./DynamicTypeText": dynamicText,
    "expo-linear-gradient": { LinearGradient: "LinearGradient" },
    "../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const { DynamicTypeText } = harness.load(
    fileURLToPath(new URL("./DynamicTypeText.tsx", import.meta.url)),
  ) as { DynamicTypeText: (props: object) => Element };
  Object.assign(dynamicText, { DynamicTypeText });
  const { PrimaryButton } = harness.load(
    fileURLToPath(new URL("./PrimaryButton.tsx", import.meta.url)),
  ) as { PrimaryButton: (props: object) => Element };
  return {
    harness,
    DynamicTypeText,
    PrimaryButton,
    scale: (next: number) => {
      fontScale = next;
      harness.render();
    },
  };
}

test("live scale changes invalidate only the native text leaf and retain scaling, text and native event props", () => {
  const env = setup();
  const onLayout = () => {};
  const props = {
    children: "Settings",
    style: { fontSize: 18 },
    onLayout,
    selectable: true,
    accessibilityRole: "header",
  };
  env.harness.mount(() => env.DynamicTypeText(props));
  assert.equal(env.harness.value().key, 1);
  env.harness.render();
  assert.equal(
    env.harness.value().key,
    1,
    "ordinary renders keep the native leaf",
  );
  for (const scale of [2.143, 1]) {
    env.scale(scale);
    const leaf = env.harness.value();
    assert.equal(leaf.type, "Text");
    assert.equal(leaf.key, scale);
    assert.equal(leaf.props.onLayout, onLayout);
    assert.equal(leaf.props.style, props.style);
    assert.equal(leaf.props.children, "Settings");
    assert.equal(leaf.props.selectable, true);
    assert.equal(leaf.props.accessibilityRole, "header");
    assert.notEqual(leaf.props.allowFontScaling, false);
    assert.equal(leaf.props.maxFontSizeMultiplier, undefined);
  }
  env.harness.unmount();
});

test("Settings button opt-in refreshes its label while its Pressable identity, disabled state and action remain", () => {
  for (const kind of ["primary", "ghost", "danger"]) {
    const env = setup();
    let presses = 0;
    const onPress = () => presses++;
    env.harness.mount(() =>
      env.PrimaryButton({
        label: "Retry",
        onPress,
        kind,
        disabled: true,
        refreshLabelOnFontScaleChange: true,
      }),
    );
    const original = env.harness.value();
    const label = () => {
      const button = env.harness.value();
      return typeof button.props.children === "function"
        ? button.props.children({ pressed: false }).props.children
        : button.props.children;
    };
    assert.equal(label().type, env.DynamicTypeText);
    env.scale(2.143);
    const current = env.harness.value();
    assert.equal(current.type, original.type);
    assert.equal(current.key, original.key);
    assert.equal(current.props.onPress, onPress);
    assert.equal(current.props.disabled, true);
    assert.equal(label().type, env.DynamicTypeText);
    assert.equal(env.DynamicTypeText(label().props).key, 2.143);
    current.props.onPress();
    assert.equal(presses, 1);
    env.harness.mount(() =>
      env.PrimaryButton({ label: "Other", onPress, kind }),
    );
    assert.equal(
      label().type,
      "Text",
      "other button consumers retain their native Text path",
    );
    env.harness.unmount();
  }
});
