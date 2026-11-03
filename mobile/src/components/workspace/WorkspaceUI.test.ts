import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { lightColors } from "../../theme/mobileTheme";

type E = { type: any; props: Record<string, any> };
const nodes = (node: E): E[] => [
  node,
  ...[node.props.children]
    .flat(Infinity)
    .flatMap((child) => (child?.props ? nodes(child) : [])),
];
const flatten = (style: unknown): Record<string, any> =>
  Array.isArray(style)
    ? Object.assign({}, ...style.map(flatten))
    : style && typeof style === "object"
      ? (style as Record<string, any>)
      : {};

function load() {
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "react-native": {
      ActivityIndicator: "ActivityIndicator",
      KeyboardAvoidingView: "KeyboardAvoidingView",
      Platform: { OS: "ios" },
      Pressable: "Pressable",
      RefreshControl: "RefreshControl",
      ScrollView: "ScrollView",
      StyleSheet: { create: (styles: object) => styles },
      Text: "Text",
      TextInput: "TextInput",
      View: "View",
    },
    "expo-linear-gradient": { LinearGradient: "LinearGradient" },
    "react-native-safe-area-context": {
      SafeAreaView: "SafeAreaView",
      useSafeAreaInsets: () => ({ top: 0 }),
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../theme/designTokens": {
      fonts: {},
      radius: { md: 8, lg: 12 },
      TOUCH_TARGET: 44,
    },
    "./AndroidKeyboardAvoidingView": { AndroidKeyboardAvoidingView: "AKAV" },
  });
  const ui = h.load(
    fileURLToPath(new URL("./WorkspaceUI.tsx", import.meta.url)),
  ) as {
    ActionButton: (props: Record<string, unknown>) => E;
    Card: (props: Record<string, unknown>) => E;
  };
  return { h, ui };
}

test("compact secondary buttons draw a 3:1 control edge instead of a borderless tint", () => {
  const { h, ui } = load();
  h.mount(() =>
    ui.ActionButton({
      label: "Chat",
      compact: true,
      variant: "secondary",
      onPress: () => {},
    }),
  );
  const fill = nodes(h.value()).find(
    (node) => node.type === "View" && flatten(node.props.style).minHeight === 36,
  )!;
  const style = flatten(fill.props.style);
  assert.equal(style.borderWidth, 1);
  assert.equal(style.borderColor, lightColors.controlBorder);
  assert.equal(style.backgroundColor, lightColors.navActive);
});

test("cards lift off the white light-mode slab with the themed shadow", () => {
  const { h, ui } = load();
  h.mount(() => ui.Card({ children: null }));
  const style = flatten(h.value().props.style);
  assert.equal(style.borderColor, lightColors.border);
  assert.equal(style.shadowColor, lightColors.shadowColor);
  assert.ok(style.shadowOpacity > 0 && style.shadowRadius > 0);
  assert.ok(style.elevation >= 1);
});
