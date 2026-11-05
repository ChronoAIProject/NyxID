import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
const flatten = (styles: any): Record<string, any> =>
  Array.isArray(styles)
    ? Object.assign({}, ...styles.map(flatten))
    : styles || {};

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

test("root legal documents share a readable column with their sticky Back control; sheets keep their own chrome", () => {
  for (const inSheet of [false, true]) {
    let backs = 0;
    const harness = hookHarness<Element>({
      "react/jsx-runtime": { jsx, jsxs: jsx },
      "react-native": {
        View: "View",
        Text: "Text",
        ScrollView: "ScrollView",
        StyleSheet: { create: <T>(styles: T) => styles },
      },
      "react-native-markdown-display": "Markdown",
      "../../app/WorkspaceHeader": {
        useWorkspaceSheet: () => (inSheet ? {} : null),
      },
      "../../components/BlurBackButton": { BlurBackButton: "BlurBackButton" },
      "../../components/ScreenContainer": {
        ScreenContainer: "ScreenContainer",
      },
      "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
      "../../theme/flowStyles": {
        createFlowStyles: () => ({ content: { flex: 1 } }),
      },
      "../../lib/telemetry": { capture: () => {} },
      "./legalDocumentUrl": { resolveLegalDocumentUrl: () => null },
    });
    const { LegalDocumentScreen } = harness.load(
      fileURLToPath(new URL("./LegalDocumentScreen.tsx", import.meta.url)),
    ) as { LegalDocumentScreen: (props: unknown) => Element };
    harness.mount(() =>
      LegalDocumentScreen({
        title: "Terms of Service",
        docKey: "terms",
        telemetryBackTarget: "auth",
        onBack: () => {
          backs++;
        },
      }),
    );
    const nodes = elements(harness.value());
    const scroller = nodes.find((item) => item.type === "ScrollView")!;
    const content = flatten(scroller.props.contentContainerStyle);
    assert.equal(
      flatten(scroller.props.style).flex,
      1,
      "the scrolling viewport still fills the screen",
    );
    assert.equal(content.maxWidth, inSheet ? undefined : 720);
    assert.equal(content.width, inSheet ? undefined : "100%");
    assert.equal(content.alignSelf, inSheet ? undefined : "center");
    const back = nodes.find((item) => item.type === "BlurBackButton");
    assert.equal(Boolean(back), !inSheet);
    if (back) {
      const column = nodes.find((item) => item.props.children === back)!;
      assert.equal(flatten(column.props.style).maxWidth, content.maxWidth);
      assert.equal(
        flatten(column.props.style).paddingHorizontal,
        content.paddingHorizontal,
      );
      back.props.onPress();
      assert.equal(backs, 1);
    }
    harness.unmount();
  }
});

test("legal Back exposes a named button and invokes its callback", () => {
  let backs = 0;
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Pressable: "Pressable",
      Text: "Text",
      StyleSheet: { create: <T>(styles: T) => styles },
    },
    "expo-blur": { BlurView: "BlurView" },
    "../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const { BlurBackButton } = harness.load(
    fileURLToPath(
      new URL("../../components/BlurBackButton.tsx", import.meta.url),
    ),
  ) as { BlurBackButton: (props: unknown) => Element };
  harness.mount(() =>
    BlurBackButton({
      onPress: () => {
        backs++;
      },
    }),
  );
  assert.equal(harness.value().type, "Pressable");
  assert.equal(harness.value().props.accessibilityRole, "button");
  assert.equal(harness.value().props.accessibilityLabel, "Back");
  harness.value().props.onPress();
  assert.equal(backs, 1);
  harness.unmount();
});
