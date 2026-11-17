import { WORKSPACE_HEADER_ROW_HEIGHT } from "./workspaceHeaderMetrics";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";

/** Static row styles may be split across an array (the slab-motion slot). */
const flatStyle = (node: { props: Record<string, any> }) =>
  Object.assign({}, ...[node.props.style].flat(Infinity).filter(Boolean));

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
      "react-native": {
      Platform: {OS: "ios"},
        Text: "Text",
        View: "View",
        useWindowDimensions: () => ({ width: 402, height: 874 }),
      },
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
    flatStyle(h.value().props.children).height,
    WORKSPACE_HEADER_ROW_HEIGHT,
  );
  assert.ok(
    Math.abs(
      62 +
        flatStyle(h.value().props.children).height / 2 -
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

test("on the folded Duo page bars sit flush at the top, left of the status column; pages keeping the rail inset start right under them", () => {
  const corner = { value: true };
  const railInset = { value: 66 };
  let window = { width: 466, height: 678 };
  const mocks: Record<string, any> = {};
  const h = hookHarness<E>(
    Object.assign(mocks, {
      "react/jsx-runtime": {
        jsx: (type: any, props: any) => ({ type, props }),
        jsxs: (type: any, props: any) => ({ type, props }),
      },
      "react-native": {
      Platform: {OS: "ios"},
        Text: "Text",
        View: "View",
        useWindowDimensions: () => window,
      },
      "react-native-safe-area-context": {
        useSafeAreaInsets: () => ({ top: 0, left: 0, right: 83 }),
      },
      "./WorkspaceLayoutContext": {
        WorkspaceCornerSensorContext: corner,
        WorkspaceQuickRailContext: {
          get value() {
            return railInset.value > 0;
          },
        },
      },
      "../../theme/ThemeContext": {
        useTheme: () => ({ colors: { bg: "white", textPrimary: "black" } }),
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
  const component = callback({
    options: { title: "Automations" },
    route: { name: "Automations" },
  });
  h.mount(() => component.type(component.props));
  // Flush at the top (8pt, was 62.5pt): row 8–68pt. The page keeps the
  // rail inset (rail shown or full screen alike), so it starts right under
  // the row (68pt, was 152pt) without reaching the status column.
  h.render();
  assert.equal(h.value().props.style.paddingTop, 8);
  assert.equal(h.value().props.style.paddingBottom, 0);
  // A full-width body would start below the cluster instead.
  railInset.value = 0;
  h.render();
  assert.equal(h.value().props.style.paddingTop, 8);
  assert.equal(h.value().props.style.paddingBottom, 152 - 68);
  railInset.value = 66;
  h.render();
  const row = () => flatStyle(h.value().props.children);
  // The composer's 12pt margin, plus the inset: trailing controls end at
  // 371pt, left of the cluster column (383pt) and its ink (397.67pt).
  assert.equal(row().paddingRight, 12 + 83);
  assert.equal(row().paddingLeft, 12);
  assert.equal(row().gap, 8);
  // Ordinary phones: the bar sits at the safe-area inset, as before.
  railInset.value = 0;
  corner.value = false;
  window = { width: 402, height: 874 };
  h.render();
  assert.equal(h.value().props.style.paddingTop, 0);
  assert.equal(h.value().props.style.paddingBottom, 0);
  h.unmount();
});

test("P9 page headers retain the base native paragraph identity", () => {
  const source = readFileSync(
    new URL("./WorkspacePageHeader.tsx", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(source, /DynamicTypeText as Text/);
  assert.match(source, /import \{[^}]*\bText\b[^}]*\} from "react-native"/);
});
