import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";

test("legacy workspace list surfaces run to the right edge beside a corner cluster; ordinary devices retain their native safe area", () => {
  const corner = { value: false };
  const jsx = (type: unknown, props: any) => ({ type, props });
  const h = hookHarness<any>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Platform: { OS: "ios" },
      SafeAreaView: "NativeSafeArea",
      View: "View",
      StyleSheet: { create: (value: unknown) => value },
    },
    "react-native-safe-area-context": { SafeAreaView: "SafeArea" },
    "./workspace/WorkspaceLayoutContext": {
      WorkspaceCornerSensorContext: corner,
    },
    "../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const { ScreenContainer } = h.load(
    fileURLToPath(new URL("./ScreenContainer.tsx", import.meta.url)),
  ) as any;
  h.mount(() => ScreenContainer({ children: "Retained list" }));
  assert.equal(h.value().type, "NativeSafeArea");
  assert.equal(h.value().props.edges, undefined);
  // Beside the Duo's status column the list runs under the floating rail.
  corner.value = true;
  h.render();
  assert.equal(h.value().type, "SafeArea");
  assert.deepEqual(Array.from(h.value().props.edges), ["bottom", "left"]);
  assert.equal(h.value().props.children.props.children, "Retained list");
  corner.value = false;
  h.render();
  assert.equal(h.value().type, "NativeSafeArea");
  assert.equal(h.value().props.children.props.children, "Retained list");
  h.unmount();
});

test("on the folded Duo legacy pages put their content in a RailInsetView (rail mode clear of the rail, full screen to the margin); the background runs underneath", () => {
  const jsx = (type: unknown, props: any) => ({ type, props });
  const h = hookHarness<any>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Platform: { OS: "ios" },
      SafeAreaView: "NativeSafeArea",
      View: "View",
      StyleSheet: { create: (value: unknown) => value },
    },
    "react-native-safe-area-context": { SafeAreaView: "SafeArea" },
    "./workspace/WorkspaceLayoutContext": {
      WorkspaceCornerSensorContext: { value: true },
    },
    "../theme/ThemeContext": { useTheme: () => ({ colors: { bg: "#fff" } }) },
  });
  const { ScreenContainer } = h.load(
    fileURLToPath(new URL("./ScreenContainer.tsx", import.meta.url)),
  ) as any;
  h.mount(() => ScreenContainer({ children: "Settings" }));
  const surface = h.value();
  const flat = (style: unknown) =>
    Object.assign({}, ...[style].flat(Infinity).filter(Boolean));
  // The background (the surface) is never narrowed.
  assert.equal(flat(surface.props.style).paddingRight, undefined);
  assert.equal(flat(surface.props.style).backgroundColor, "#fff");
  // The content animates the rail inset (WorkspaceSlabMotion's RailInsetView).
  const content = surface.props.children;
  assert.equal(content.type, "RailInsetView");
  assert.equal(content.props.ownPaddingRight, undefined);
  assert.equal(flat(content.props.style).flex, 1);
  assert.equal(flat(content.props.style).paddingRight, undefined);
  assert.equal(content.props.children, "Settings");
  h.unmount();
});

test("Android content protects its bottom navigation inset using the cross-platform safe area", () => {
  const jsx = (type: unknown, props: any) => ({ type, props });
  const h = hookHarness<any>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": { Platform: { OS: "android" }, SafeAreaView: "NativeSafeArea", StyleSheet: { create: (value: unknown) => value } },
    "react-native-safe-area-context": { SafeAreaView: "SafeArea" },
    "./workspace/WorkspaceLayoutContext": { WorkspaceCornerSensorContext: { value: false } },
    "../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const { ScreenContainer } = h.load(fileURLToPath(new URL("./ScreenContainer.tsx", import.meta.url))) as any;
  h.mount(() => ScreenContainer({ children: "Delete Chat" }));
  assert.equal(h.value().type, "SafeArea", "RN's native SafeAreaView does not apply Android navigation insets");
  assert.deepEqual(Array.from(h.value().props.edges), ["bottom", "left", "right"]);
  assert.equal(h.value().props.children.props.children, "Delete Chat");
  h.unmount();
});
