import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { workspaceAccessibilityState } from "./workspaceAccessibility";

test("root drawer state excludes retained Android chat and header descendants even while their navigator reports focused", () => {
  let sheetOpen = false;
  let focused = true;
  let inSheet = false;
  const child = { type: "RetainedChat", props: {} };
  const harness = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": { View: "View" },
    "@react-navigation/native": { useIsFocused: () => focused },
    "./workspaceAccessibility": {
      useWorkspaceAccessibility: () => ({ sheetOpen }),
    },
  });
  const Surface = harness.load(
    fileURLToPath(
      new URL("./WorkspaceInteractionSurface.tsx", import.meta.url),
    ),
  ).WorkspaceInteractionSurface as (props: {
    children: unknown;
    inSheet: boolean;
  }) => any;
  harness.mount(() => Surface({ children: child, inSheet }));
  assert.equal(harness.value().props.importantForAccessibility, "auto");
  sheetOpen = true;
  harness.render();
  assert.equal(
    harness.value().props.importantForAccessibility,
    "no-hide-descendants",
  );
  assert.equal(harness.value().props.accessibilityElementsHidden, true);
  assert.equal(harness.value().props.children, child, "chat remains mounted");
  inSheet = true;
  harness.render();
  assert.equal(harness.value().props.importantForAccessibility, "auto");
  inSheet = false;
  sheetOpen = false;
  harness.render();
  assert.equal(harness.value().props.importantForAccessibility, "auto");
  focused = false;
  harness.render();
  assert.equal(
    harness.value().props.importantForAccessibility,
    "no-hide-descendants",
  );
  harness.unmount();
});
test("root accessibility classification tracks cold sheets, contextual chats and auth transitions", () => {
  const chat = { name: "GroupChat", key: "group" };
  const sheet = { name: "WorkspaceSheet", key: "sheet" };
  assert.deepEqual(
    workspaceAccessibilityState({
      stale: true,
      routes: [chat, sheet],
      index: 1,
    }),
    { sheetOpen: true, primaryKey: "group" },
  );
  assert.deepEqual(
    workspaceAccessibilityState({ stale: true, routes: [chat], index: 0 }),
    { sheetOpen: false, primaryKey: "group" },
  );
  assert.deepEqual(workspaceAccessibilityState(undefined), {
    sheetOpen: false,
    primaryKey: undefined,
  });
  assert.deepEqual(
    workspaceAccessibilityState({
      stale: true,
      routes: [{ name: "Auth", key: "auth" }],
      index: 0,
    }),
    { sheetOpen: false, primaryKey: "auth" },
  );
});
