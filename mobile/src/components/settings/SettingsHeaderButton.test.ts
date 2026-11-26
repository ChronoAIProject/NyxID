import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";

type E = { type: unknown; props: Record<string, any> };
// The harness evaluates the module in its own realm: compare objects as JSON.
const same = (actual: unknown, expected: unknown) =>
  assert.equal(JSON.stringify(actual), JSON.stringify(expected));

function render(props: Record<string, unknown>) {
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      ActivityIndicator: "ActivityIndicator",
      StyleSheet: { create: (v: unknown) => v },
    },
    "../DynamicTypeText": { DynamicTypeText: "Text" },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primary: "#7C3AED" } }),
    },
  });
  const { SettingsHeaderButton } = h.load(
    fileURLToPath(new URL("./SettingsHeaderButton.tsx", import.meta.url)),
  ) as { SettingsHeaderButton: (props: unknown) => E };
  h.mount(() => SettingsHeaderButton(props));
  const value = h.value();
  h.unmount();
  return value;
}

test("an edit page's commit action is the bars' floating glass pill hugging its label", () => {
  const presses: string[] = [];
  const button = render({ enabled: true, onPress: () => presses.push("save") });
  // The same primitive as Menu/Back and the chat bar's controls: standard
  // glass, a 44pt minimum from the primitive, only its own padding here.
  assert.equal(button.type, "FloatingButton");
  assert.equal(button.props.capsuleGlass, undefined, "standard glass");
  same(button.props.style, { paddingHorizontal: 14 });
  assert.equal(button.props.accessibilityLabel, "Save");
  same(button.props.accessibilityState, { disabled: false, busy: false });
  assert.equal(button.props.disabled, false);
  const label = button.props.children as E;
  assert.equal(label.type, "Text");
  assert.equal(label.props.children, "Save");
  assert.equal([label.props.style].flat().at(-1).color, "#7C3AED");
  button.props.onPress();
  same(presses, ["save"]);
});

test("disabled dims through the primitive; busy keeps its spinner in the same capsule and takes no press", () => {
  const disabled = render({ label: "Done", enabled: false, onPress() {} });
  assert.equal(disabled.type, "FloatingButton");
  assert.equal(disabled.props.disabled, true);
  same(disabled.props.accessibilityState, { disabled: true, busy: false });
  assert.equal((disabled.props.children as E).props.children, "Done");
  const busy = render({ enabled: true, busy: true, onPress() {} });
  // The capsule stays (no jump in the bar); the spinner replaces the label,
  // undimmed, and a second save cannot start.
  assert.equal(busy.type, "FloatingButton");
  assert.equal((busy.props.children as E).type, "ActivityIndicator");
  assert.equal(busy.props.disabled, false);
  assert.equal(busy.props.onPress, undefined);
  same(busy.props.accessibilityState, { disabled: true, busy: true });
});
