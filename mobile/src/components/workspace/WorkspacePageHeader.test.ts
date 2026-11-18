import { WORKSPACE_HEADER_ROW_HEIGHT } from "./workspaceHeaderMetrics";
import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";

type E = { type: any; props: Record<string, any> };
const nodes = (node: E): E[] => [
  node,
  ...[node.props.children]
    .flat(Infinity)
    .flatMap((child) => (child?.props ? nodes(child) : [])),
];
test("the main header callback is hook-safe; capped centered titles and custom chat identity share safe-area controls", () => {
  let themeReads = 0;
  const mocks: Record<string, any> = {};
  const h = hookHarness<E>(
    Object.assign(mocks, {
      "react/jsx-runtime": {
        jsx: (type: any, props: any) => ({ type, props }),
        jsxs: (type: any, props: any) => ({ type, props }),
      },
      "react-native": { Text: "Text", View: "View" },
      "react-native-safe-area-context": {
        useSafeAreaInsets: () => ({ top: 62 }),
      },
      "../../theme/ThemeContext": {
        useTheme: () => {
          themeReads++;
          return { colors: { bg: "white", textPrimary: "black" } };
        },
      },
      "../../theme/designTokens": { fonts: { displaySemi: "semi" } },
    }),
  );
  mocks["./workspaceHeaderMetrics"] = h.load(
    fileURLToPath(new URL("./workspaceHeaderMetrics.ts", import.meta.url)),
  );
  const callback = h.load(
    fileURLToPath(new URL("./WorkspacePageHeader.tsx", import.meta.url)),
  ).WorkspacePageHeader as (props: any) => E;
  const options = {
    title: "Owner policy and updates",
    headerLeft: () => ({ type: "Back44", props: {} }),
    headerRight: () => ({ type: "Action44", props: {} }),
  };
  const component = callback({ options, route: { name: "MachinePolicy" } });
  assert.equal(themeReads, 0, "navigator option invocation calls no hooks");
  h.mount(() => component.type(component.props));
  const text = nodes(h.value()).find((node) => node.type === "Text")!;
  assert.equal(text.props.numberOfLines, 1);
  assert.equal(text.props.maxFontSizeMultiplier, 1.25);
  assert.equal(text.props.style.fontSize, 18);
  assert.equal(text.props.children, "Owner policy and updates");
  assert.equal(text.props.style.textAlign, "center");
  const byId = (id: string) =>
    nodes(h.value()).find((n) => n.props.testID === id)!;
  byId("workspace-header-trailing").props.children.props.onLayout({
    nativeEvent: { layout: { width: 88 } },
  });
  h.render();
  assert.equal(byId("workspace-header-leading").props.style.width, 88);
  assert.equal(byId("workspace-header-trailing").props.style.width, 88);
  for (const width of [320, 402, 1024]) {
    const left = 8 + 88 + 4,
      right = width - 8 - 88 - 4;
    assert.ok(
      Math.abs((left + right) / 2 - width / 2) <= 0.5,
      "title is centered on full header width",
    );
  }
  assert.equal(h.value().props.style.paddingTop, 62);
  assert.equal(
    h.value().props.children.props.style.height,
    WORKSPACE_HEADER_ROW_HEIGHT,
  );
  assert.ok(
    Math.abs(
      62 +
        h.value().props.children.props.style.height / 2 -
        (62 + WORKSPACE_HEADER_ROW_HEIGHT / 2),
    ) <= 0.5,
  );
  assert.ok(nodes(h.value()).some((node) => node.type === "Back44"));
  const chat = callback({
    options: {
      ...options,
      headerTitle: () => ({ type: "AgentHeaderTitle", props: {} }),
    },
    route: { name: "Assistant" },
  });
  h.unmount();
  h.mount(() => chat.type(chat.props));
  assert.ok(nodes(h.value()).some((node) => node.type === "AgentHeaderTitle"));
  assert.equal(
    nodes(h.value()).some((node) => node.type === "Text"),
    false,
  );
  h.unmount();
});
