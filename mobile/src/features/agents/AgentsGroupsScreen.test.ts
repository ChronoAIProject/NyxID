import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
function elements(root: Element): Element[] {
  if (typeof root.type === "function") return elements(root.type(root.props));
  return [
    root,
    ...[root.props.children, root.props.listHeader]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

function setup(tab: "agents" | "groups", online = true) {
  const created: string[] = [];
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      Pressable: "Pressable",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": { Plus: "Plus" },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: online }),
    },
    "../../components/workspace/WorkspaceUI": {
      Label: "Label",
      WorkspacePage: "WorkspacePage",
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primaryTone: {} } }),
    },
    "../../theme/designTokens": { fonts: {} },
    "./AgentsScreen": { AgentsScreen: "AgentsScreen" },
    "../groups/GroupsList": { GroupsList: "GroupsList" },
  });
  const { AgentsGroupsScreen } = harness.load(
    fileURLToPath(new URL("./AgentsGroupsScreen.tsx", import.meta.url)),
  ) as {
    AgentsGroupsScreen: (props: object) => Element;
  };
  harness.mount(() =>
    AgentsGroupsScreen({
      tab,
      onTabChange: () => {},
      onOpenGroup: () => {},
      onCreateAgent: () => created.push("agent"),
      onCreateGroup: () => created.push("group"),
    }),
  );
  const all = elements(harness.flush());
  const button = all.find(
    (row) =>
      row.props.accessibilityLabel ===
      (tab === "agents" ? "New agent" : "New crew"),
  );
  assert.ok(button);
  return { created, button, all };
}

test("the active hub tab exposes its creation action at the top", () => {
  for (const tab of ["agents", "groups"] as const) {
    const env = setup(tab);
    assert.equal(env.button.props.disabled, false);
    env.button.props.onPress();
    assert.deepEqual(env.created, [tab === "agents" ? "agent" : "group"]);
    const list = env.all.find(
      (row) => row.type === (tab === "agents" ? "AgentsScreen" : "GroupsList"),
    );
    assert.ok(list);
    list.props.onCreate();
    assert.equal(
      env.created.length,
      2,
      "the existing list action reaches the same route",
    );
  }
});

test("hub creation actions are disabled while offline", () => {
  for (const tab of ["agents", "groups"] as const) {
    const env = setup(tab, false);
    assert.equal(env.button.props.disabled, true);
    assert.equal(env.button.props.accessibilityState.disabled, true);
    assert.deepEqual(env.created, []);
  }
});
