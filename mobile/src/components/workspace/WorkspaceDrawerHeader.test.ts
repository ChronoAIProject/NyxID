import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
test("nested drawer headers keep Back/title/Close in a centered 18pt row", () => {
  const mocks: Record<string, any> = {};
  const h = hookHarness<any>(
    Object.assign(mocks, {
      "react/jsx-runtime": {
        jsx: (type: any, props: any) => ({ type, props }),
        jsxs: (type: any, props: any) => ({ type, props }),
      },
      "react-native-safe-area-context": {
        useSafeAreaInsets: () => ({ left: 0, right: 0 }),
      },
      "react-native": {
        View: "View",
        Text: "Text",
        StyleSheet: { create: (v: any) => v },
      },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    }),
  );
  mocks["./workspaceHeaderMetrics"] = h.load(
    fileURLToPath(new URL("./workspaceHeaderMetrics.ts", import.meta.url)),
  );
  const Header = h.load(
    fileURLToPath(new URL("./WorkspaceDrawerHeader.tsx", import.meta.url)),
  ).WorkspaceDrawerHeader as any;
  h.mount(() =>
    (() => {
      const element = Header({
        back: {},
        navigation: {
          getState: () => ({
            index: 1,
            routes: [{ key: "root" }, { key: "usage" }],
          }),
        },
        route: { name: "UsageCredits", key: "usage" },
        options: {
          title: "Usage & credits",
          headerLeft: () => "Back",
          headerRight: () => "Close",
        },
      });
      return element.type(element.props);
    })(),
  );
  // The sheet's band wraps the 60pt row.
  assert.equal(h.value().props.testID, "workspace-sheet-header-band");
  const [back, title, close] = h.value().props.children.props.children;
  assert.equal(back.props.children.props.children, "Back");
  assert.equal(close.props.children.props.children, "Close");
  assert.equal(back.props.style.width, close.props.style.width);
  assert.equal(title.props.children, "Usage & credits");
  assert.equal(title.props.style[0].fontSize, 18);
  assert.equal(title.props.style[0].flex, 1);
  assert.equal(title.props.style[0].textAlign, "center");
  assert.equal(title.props.numberOfLines, 1);
  assert.equal(title.props.maxFontSizeMultiplier, 1.25);
  h.unmount();
  const source = readFileSync(
    fileURLToPath(new URL("../../app/AppNavigator.tsx", import.meta.url)),
    "utf8",
  );
  const drawer = source.slice(
    source.indexOf("<SheetStack.Navigator"),
    source.indexOf("</SheetStack.Navigator>"),
  );
  const defaults = drawer.slice(0, drawer.indexOf("<SheetStack.Screen"));
  assert.match(defaults, /header: WorkspaceDrawerHeader/);
  assert.match(
    drawer,
    /name="UsageCredits"[\s\S]*?header: WorkspaceDrawerHeader/,
  );
  for (const override of drawer.matchAll(/header:\s*(\w+)/g))
    assert.equal(override[1], "WorkspaceDrawerHeader");
});

test("drawer header options return an element without borrowing navigator hooks", () => {
  let calls = 0;
  const h = hookHarness<any>({
    "react/jsx-runtime": { jsx: (type: any, props: any) => ({ type, props }) },
    "react-native-safe-area-context": {
      useSafeAreaInsets: () => ({ left: 0, right: 0 }),
    },
    "react-native": { StyleSheet: { create: (value: any) => value } },
    "../../theme/ThemeContext": {
      useTheme: () => {
        calls++;
        throw new Error("Hooks must run only when React renders the child");
      },
    },
  });
  const Header = h.load(
    fileURLToPath(new URL("./WorkspaceDrawerHeader.tsx", import.meta.url)),
  ).WorkspaceDrawerHeader as any;
  for (const name of ["Machines", "UsageCredits", "GroupEditor", "Machines"]) {
    const element = Header({ route: { name }, options: { title: name } });
    assert.equal(calls, 0);
    assert.equal(typeof element.type, "function");
    assert.equal(element.props.route.name, name);
  }
});

test("sheet Back depends on the local route index, never the surrounding app history", () => {
  for (const [name, index] of [
    ["DeviceLogin", 0],
    ["AgentKeyLogin", 0],
    ["AccountSettings", 0],
    ["UsageCredits", 0],
    ["AccountSecurity", 1],
    ["UsageCredits", 1],
  ] as const) {
    const mocks: Record<string, any> = {
      "react/jsx-runtime": {
        jsx: (type: any, props: any) => ({ type, props }),
        jsxs: (type: any, props: any) => ({ type, props }),
      },
      "react-native-safe-area-context": {
        useSafeAreaInsets: () => ({ left: 0, right: 0 }),
      },
      "react-native": {
        View: "View",
        Text: "Text",
        StyleSheet: { create: (v: any) => v },
      },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    };
    const h = hookHarness<any>(mocks);
    mocks["./workspaceHeaderMetrics"] = h.load(
      fileURLToPath(new URL("./workspaceHeaderMetrics.ts", import.meta.url)),
    );
    const Header = h.load(
      fileURLToPath(new URL("./WorkspaceDrawerHeader.tsx", import.meta.url)),
    ).WorkspaceDrawerHeader as any;
    const routes = index
      ? [{ key: "settings" }, { key: name }]
      : [{ key: name }];
    h.mount(() => {
      const e = Header({
        navigation: { getState: () => ({ index, routes }) },
        route: { name, key: name },
        back: { title: "global history" },
        options: {
          title: name,
          headerLeft: () => "Back",
          headerRight: () => "Close",
        },
      });
      return e.type(e.props);
    });
    const [left, , right] = h.value().props.children.props.children;
    assert.equal(
      left.props.children.props.children,
      index > 0 ? "Back" : null,
      name + " local stack index " + index,
    );
    assert.equal(right.props.children.props.children, "Close");
    h.unmount();
  }
});

test("sheet header actions retain live sensor and landscape insets without padding the body", () => {
  let insets = { left: 0, right: 0 };
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Text: "Text",
      StyleSheet: { create: (v: any) => v },
    },
    "react-native-safe-area-context": { useSafeAreaInsets: () => insets },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./workspaceHeaderMetrics": {
      WORKSPACE_HEADER_ROW: {},
      FLOATING_HEADER_MARGIN: 12,
      FLOATING_HEADER_GAP: 8,
      useWorkspaceHeaderSides: () => ({ width: 44 }),
    },
  });
  const { WorkspaceDrawerHeader } = h.load(
    fileURLToPath(new URL("./WorkspaceDrawerHeader.tsx", import.meta.url)),
  ) as any;
  h.mount(() => {
    const element = WorkspaceDrawerHeader({
      navigation: { getState: () => ({ routes: [{ key: "settings" }] }) },
      route: { name: "AccountSettings", key: "settings" },
      options: { title: "Settings", headerRight: () => "Close" },
    });
    return element.type(element.props);
  });
  for (const value of [
    { left: 0, right: 104 },
    { left: 59, right: 0 },
    { left: 0, right: 0 },
  ]) {
    insets = value;
    h.render();
    const row = h.value().props.children;
    const style = Object.assign({}, ...row.props.style);
    // The bars' shared 12pt margin and 8pt gap between floating controls.
    assert.equal(style.paddingRight, 12 + value.right);
    assert.equal(style.paddingLeft, 12 + value.left);
    assert.equal(style.gap, 8);
    assert.equal(row.props.children[2].props.children.props.style.gap, 8);
    assert.equal(
      row.props.children[2].props.children.props.children,
      "Close",
    );
  }
  h.unmount();
});
