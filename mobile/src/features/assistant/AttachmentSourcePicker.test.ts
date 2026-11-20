import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

test("attachment sources form full-width growing rows and launch only after dismissal", () => {
  type Element = { type: unknown; props: Record<string, any> };
  const nodes = (node: any): Element[] =>
    node?.props
      ? [node, ...[node.props.children].flat(Infinity).flatMap(nodes)]
      : [];
  let selected = 0;
  let closed = 0;
  let pending: (() => void) | undefined;
  let finish: () => void = () => {};
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: any) => ({ type, props }),
      jsxs: (type: unknown, props: any) => ({ type, props }),
    },
    "react-native": {
      ...Object.fromEntries(
        ["Modal", "Pressable", "ScrollView", "View"].map(name => [name, name]),
      ),
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "react-native-safe-area-context": { SafeAreaView: "SafeAreaView" },
    "../../components/DynamicTypeText": { DynamicTypeText: "Text" },
    "../../components/workspace/useNativeSheetDismiss": {
      useNativeSheetDismiss: (onClosed: () => void) => {
        finish = () => { pending?.(); onClosed(); };
        return {
          modalProps: { visible: true },
          close: (after?: () => void) => { pending = after; },
        };
      },
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const Picker = h.load(
    fileURLToPath(new URL("./AttachmentSourcePicker.tsx", import.meta.url)),
  ).AttachmentSourcePicker as (props: Parameters<typeof import("./AttachmentSourcePicker").AttachmentSourcePicker>[0]) => Element;
  h.mount(() => Picker({
    choices: ["Camera", "Photos", "Files"].map(label => ({
      label,
      onPress: () => selected++,
    })),
    onClose: () => closed++,
  }));
  const all = nodes(h.value());
  const buttons = all.filter(node => node.type === "Pressable");
  assert.equal(buttons.length, 4);
  for (const button of buttons) {
    assert.equal(button.props.style.height, undefined);
    assert.equal(button.props.style.minHeight, 48);
    assert.equal(button.props.style.alignSelf, "stretch");
  }
  assert.ok(all.some(node => node.type === "ScrollView"));
  buttons[2]!.props.onPress();
  assert.equal(selected, 0);
  finish();
  assert.equal(selected, 1);
  assert.equal(closed, 1);
  buttons[3]!.props.onPress();
  finish();
  assert.equal(selected, 1);
  assert.equal(closed, 2);
  h.unmount();
});
