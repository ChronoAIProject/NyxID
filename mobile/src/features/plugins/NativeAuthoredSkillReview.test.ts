import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };

function find(
  root: Element,
  predicate: (element: Element) => boolean,
): Element {
  if (predicate(root)) return root;
  for (const child of [root.props.children].flat(Infinity)) {
    if (child && typeof child === "object" && "props" in child) {
      try {
        return find(child, predicate);
      } catch {
        // Keep looking through siblings.
      }
    }
  }
  throw new Error("Element not found");
}

function setup() {
  const decision = deferred<unknown>();
  const query = {
    isFetching: false,
    isError: false,
    refetch: async () => {},
  };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove: () => {} }),
      },
      Modal: "Modal",
      View: "View",
      Text: "Text",
      Pressable: "Pressable",
      ScrollView: "ScrollView",
      ActivityIndicator: "ActivityIndicator",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "react-native-safe-area-context": {
      SafeAreaProvider: "SafeAreaProvider",
      SafeAreaView: "SafeAreaView",
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@tanstack/react-query": {
      useQueryClient: () => ({ invalidateQueries: async () => {} }),
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        ...query,
        data: queryKey.includes("authored-review")
          ? {
              state: "active",
              status: "pending",
              name: "QA skill",
              version: "1.0.0",
              files: [],
            }
          : {
              proposals: [
                { id: "proposal", revision: 1, draft: { name: "QA skill" } },
              ],
            },
      }),
    },
    "lucide-react-native": { ChevronLeft: "ChevronLeft", X: "X" },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Label", "Notice"].map((name) => [name, name]),
    ),
    "../../lib/api/pluginSkillsApi": { pluginSkillsApi: {} },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../theme/designTokens": { fonts: {} },
    "./PluginSkills": { SkillDraftEditor: "SkillDraftEditor" },
    "./authoredSkillReviewState": {
      AUTHORED_SKILL_ACTIONS: [{ action: "allow", label: "Allow publication" }],
      authoredReviewMatches: () => true,
      supportedAuthoredSkillActions: () => ["allow"],
      authoredSkillActionRequest: () => ({ kind: "decision", choice: "allow" }),
    },
  });
  const { NativeAuthoredSkillReview } = harness.load(
    fileURLToPath(new URL("./NativeAuthoredSkillReview.tsx", import.meta.url)),
  ) as { NativeAuthoredSkillReview: (props: unknown) => Element };
  harness.mount(() =>
    NativeAuthoredSkillReview({
      initiallyOpen: true,
      acknowledgement: {
        id: "acknowledgement",
        authored_skill: { agent_id: "agent", proposal_id: "proposal" },
      },
      onDecision: () => decision.promise,
    }),
  );
  const modal = () => find(harness.flush(), (item) => item.type === "Modal");
  return {
    harness,
    decision,
    modal,
    button: (label: string) =>
      find(harness.flush(), (item) => item.props.label === label),
    header: (label: string) =>
      find(harness.flush(), (item) => item.props.accessibilityLabel === label),
    editor: () =>
      find(harness.flush(), (item) => item.type === "SkillDraftEditor"),
  };
}

test("Android modal Back in authored Edit returns to the confirmation first", () => {
  const env = setup();
  env.header("Close skill confirmation");
  assert.throws(() => env.header("Back to chat"), /Element not found/);
  env.button("Edit before publishing").props.onPress();
  env.header("Back to skill confirmation");
  env.modal().props.onRequestClose();
  assert.equal(env.modal().props.visible, true);
  env.header("Close skill confirmation");
  assert.throws(
    () => env.header("Back to skill confirmation"),
    /Element not found/,
  );
  env.modal().props.onRequestClose();
  assert.equal(env.modal().props.visible, false);
  env.harness.unmount();
});

test("editor submission blocks modal Back synchronously, before the busy render", () => {
  const env = setup();
  env.button("Edit before publishing").props.onPress();
  const modal = env.modal();
  const editor = env.editor();
  editor.props.onBusy(true);
  modal.props.onRequestClose();
  assert.equal(env.modal().props.visible, true);
  assert.equal(env.header("Back to skill confirmation").props.disabled, true);
  editor.props.onBusy(false);
  env.modal().props.onRequestClose();
  env.header("Close skill confirmation");
  assert.throws(
    () => env.header("Back to skill confirmation"),
    /Element not found/,
  );
  assert.equal(env.modal().props.visible, true);
  env.harness.unmount();
});

test("an in-flight publication blocks Back and Close until its response refresh settles", async () => {
  const env = setup();
  const modal = env.modal();
  const close = env.header("Close skill confirmation");
  env.button("Allow publication").props.onPress();
  modal.props.onRequestClose();
  close.props.onPress();
  assert.equal(env.modal().props.visible, true);
  assert.equal(env.header("Close skill confirmation").props.disabled, true);
  env.decision.resolve(undefined);
  await env.harness.settle();
  env.modal().props.onRequestClose();
  assert.equal(env.modal().props.visible, false);
  env.harness.unmount();
});
