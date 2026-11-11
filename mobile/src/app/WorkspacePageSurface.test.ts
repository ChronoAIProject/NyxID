import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";

type E = { type: unknown; props: Record<string, any> };
const tick = async () => {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
};
test("slab root is opaque and reports paint only after layout and a complete frame", async () => {
  const painted: string[] = [];
  let focused = true;
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": { View: "View" },
    "@react-navigation/native": { useIsFocused: () => focused },
    "../theme/ThemeContext": {
      useTheme: () => ({ colors: { bg: "#18181B" } }),
    },
    "../lib/accessibility/WorkspaceInteractionSurface": {
      WorkspaceInteractionSurface: "Interaction",
    },
  });
  const Surface = h.load(
    fileURLToPath(new URL("./WorkspacePageSurface.tsx", import.meta.url)),
  ).WorkspacePageSurface as (props: any) => E;
  let identity = "Home";
  h.mount(() =>
    Surface({ identity, onPainted: (id: string) => painted.push(id) }),
  );
  await tick();
  assert.deepEqual(painted, [], "mount without native layout is insufficient");
  const view = h.value().props.children;
  assert.equal(view.props.style.opacity, 1);
  assert.equal(view.props.style.backgroundColor, "#18181B");
  view.props.onLayout();
  assert.deepEqual(painted, []);
  await tick();
  assert.deepEqual(painted, ["Home"]);
  identity = "Machines";
  h.render();
  assert.deepEqual(painted, ["Home"]);
  await tick();
  assert.deepEqual(painted, ["Home", "Machines"]);
  focused = false;
  h.render();
  focused = true;
  h.render();
  await tick();
  assert.deepEqual(
    painted,
    ["Home", "Machines", "Machines"],
    "a retained root repaints when it becomes focused again",
  );
  identity = "stale";
  h.render();
  h.unmount();
  await tick();
  assert.deepEqual(painted, ["Home", "Machines", "Machines"]);
});
