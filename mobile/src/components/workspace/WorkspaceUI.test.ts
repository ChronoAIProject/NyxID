import assert from "node:assert/strict";
import { floatingScrollIndicatorProps } from "./workspaceTrailingInset";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { lightColors } from "../../theme/mobileTheme";
import * as designTokens from "../../theme/designTokens";
import { readFileSync } from "node:fs";
import {
  WORKSPACE_TITLE_MAX_FONT_SCALE,
  WORKSPACE_TITLE_MAX_LINES,
} from "./workspaceHeaderMetrics";

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

function load(floatingInset = 0, fullScreen = false, cornerWindow = false) {
  const h = hookHarness<E>({
    "./WorkspaceLayoutContext": {
      WorkspaceCornerSensorContext: { value: cornerWindow },
    },
    "./workspaceFloatingInset": {
      useRailScrollIndicator: () =>
        floatingScrollIndicatorProps(floatingInset, fullScreen),
    },
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
    "../../theme/designTokens": designTokens,
    "./ScrollEdgeFades": { PageScrollView: "ScrollView" },
    "./KeyboardViewport": { KeyboardViewport: "KeyboardViewport" },
  });
  const ui = h.load(
    fileURLToPath(new URL("./WorkspaceUI.tsx", import.meta.url)),
  ) as {
    ActionButton: (props: Record<string, unknown>) => E;
    Card: (props: Record<string, unknown>) => E;
    WorkspacePage: (props: Record<string, unknown>) => E;
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

test("every page's content sits in a RailInsetView: the rail inset animates on top of its own padding while the background runs underneath", () => {
  // Scrolling pages: the scroller itself narrows past the rail (and widens
  // in full screen); its content keeps only its own padding.
  {
    const { h, ui } = load(66);
    h.mount(() => ui.WorkspacePage({ title: "Automations", children: "Rows" }));
    const inset = nodes(h.value()).find((n) => n.type === "RailInsetView")!;
    assert.equal(inset.props.ownPaddingRight, undefined);
    assert.equal(flatten(inset.props.style).flex, 1);
    const scroller = [inset.props.children].flat()[0] as E;
    assert.equal(scroller.type, "ScrollView");
    const content = flatten(scroller.props.contentContainerStyle);
    assert.equal(content.padding, 20);
    assert.equal(content.paddingRight, undefined, "no static rail inset");
    // The page itself (its background) is never narrowed.
    assert.equal(flatten(h.value().props.style).marginRight, undefined);
    h.unmount();
  }
  // Fixed pages: the content view animates its own 20pt plus the inset.
  {
    const { h, ui } = load(66);
    h.mount(() =>
      ui.WorkspacePage({ title: "Automations", scroll: false, children: "Rows", }),
    );
    const inset = nodes(h.value()).find((n) => n.type === "RailInsetView")!;
    assert.equal(inset.props.ownPaddingRight, 20);
    assert.equal(flatten(inset.props.style).gap, 16);
    assert.equal(flatten(inset.props.style).paddingRight, undefined);
    h.unmount();
  }
  // Chat pages zero their horizontal padding; the inset still animates.
  const { h, ui } = load(66);
  h.mount(() =>
    ui.WorkspacePage({
      title: "Chat",
      scroll: false,
      showHeading: false,
      contentStyle: { paddingHorizontal: 0, paddingTop: 0 },
      children: "Transcript",
    }),
  );
  const chat = nodes(h.value()).find((n) => n.type === "RailInsetView")!;
  assert.equal(chat.props.ownPaddingRight, 0);
  assert.equal(flatten(chat.props.style).paddingHorizontal, 0);
  h.unmount();
});

test("the page scroller spreads the shared rail scroll props: indicator clear of the rail, or at the window edge in full screen with no top inset", () => {
  for (const [inset, fullScreen, expected] of [
    [0, false, {}],
    [66, false, { scrollIndicatorInsets: { right: 4 }, automaticallyAdjustsScrollIndicatorInsets: false, },],
    [66, true, { scrollIndicatorInsets: { right: 0 }, automaticallyAdjustsScrollIndicatorInsets: false, },],
  ] as const) {
    const { h, ui } = load(inset, fullScreen);
    h.mount(() => ui.WorkspacePage({ title: "Automations", children: "Rows" }));
    const scroller = nodes(h.value()).find((n) => n.type === "ScrollView")!;
    for (const [key, value] of Object.entries(expected))
      assert.equal(JSON.stringify(scroller.props[key]), JSON.stringify(value), `${inset} ${fullScreen} ${key}`,
      );
    assert.equal(
      scroller.props.contentInset,
      undefined,
      "content keeps its natural top",
    );
    h.unmount();
  }
});

test("cards have no rail inset of their own: their container provides it", () => {
  const { h, ui } = load(66);
  h.mount(() => ui.Card({ children: null }));
  const style = flatten(h.value().props.style);
  assert.equal(style.padding, 16);
  assert.equal(style.paddingRight, undefined);
  h.unmount();
});

test("Duo page's large title grows like the header bar, takes two lines at most and then ends in an ellipsis", () => {
  assert.equal(WORKSPACE_TITLE_MAX_FONT_SCALE, 1.25);
  assert.equal(WORKSPACE_TITLE_MAX_LINES, 2);
  const { h, ui } = load(0, false, true);
  h.mount(() =>
    ui.WorkspacePage({
      title: "Group participants and agents",
      children: "Body",
    }),
  );
  const title = nodes(h.value()).find(
    (node) => node.props.accessibilityRole === "header",
  )!;
  assert.equal(title.props.children, "Group participants and agents");
  assert.equal(title.props.accessibilityLabel, "Group participants and agents");
  assert.equal(title.props.numberOfLines, WORKSPACE_TITLE_MAX_LINES);
  assert.equal(title.props.ellipsizeMode, "tail");
  // The same cap as the header bar, so no title word outgrows the line and
  // breaks inside itself; growth is capped, never shrunk to fit.
  assert.equal(
    title.props.maxFontSizeMultiplier,
    WORKSPACE_TITLE_MAX_FONT_SCALE,
  );
  assert.equal(title.props.adjustsFontSizeToFit, undefined);
  h.unmount();
  // Live text-size changes re-measure the title (DynamicTypeText), and the
  // header bar shares the cap constant.
  const page = readFileSync(
    new URL("./WorkspaceUI.tsx", import.meta.url),
    "utf8",
  );
  assert.match(
    page,
    /numberOfLines=\{\s*cornerSensor\s*\?\s*WORKSPACE_TITLE_MAX_LINES\s*:\s*titleNumberOfLines\s*\}/,
  );
  const bar = readFileSync(
    new URL("./WorkspacePageHeader.tsx", import.meta.url),
    "utf8",
  );
  assert.match(bar, /maxFontSizeMultiplier=\{WORKSPACE_TITLE_MAX_FONT_SCALE\}/);
});

test("P9 ordinary page title retains uncapped base scaling and caller wrapping", () => {
  const { h, ui } = load();
  h.mount(() =>
    ui.WorkspacePage({
      title: "Ordinary title",
      titleNumberOfLines: 3,
      children: "Body",
    }),
  );
  const title = nodes(h.value()).find(
    (n) => n.props.accessibilityRole === "header",
  )!;
  assert.equal(title.props.maxFontSizeMultiplier, undefined);
  assert.equal(title.props.numberOfLines, 3);
  h.unmount();
});
