import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
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
        /* Search siblings. */
      }
    }
  }
  throw new Error("Element not found");
}
const params = {
  agentId: "agent",
  conversationId: "thread",
  acknowledgementId: "acknowledgement",
};
const acknowledgement = {
  id: params.acknowledgementId,
  authored_skill: { agent_id: "agent", proposal_id: "proposal" },
};
function setup(kind: "review" | "launcher" | "page" = "review") {
  const decision = deferred<unknown>();
  const pushed: { name: string; params: unknown }[] = [];
  let guard: any;
  const query = { isFetching: false, isError: false, refetch: async () => {} };
  const history = { acknowledgements: [acknowledgement] };
  const navigation = {
    push: (name: string, params: unknown) =>
      pushed.push(JSON.parse(JSON.stringify({ name, params }))),
    goBack: () => {},
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
      View: "View",
      ScrollView: "ScrollView",
      ActivityIndicator: "ActivityIndicator",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "@react-navigation/native": {
      useIsFocused: () => true,
      useNavigation: () => navigation,
    },
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
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Label", "Notice", "WorkspacePage"].map(
        (name) => [name, name],
      ),
    ),
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: (options: unknown) => {
        guard = options;
      },
    },
    "../assistant/useAssistant": {
      useAssistant: (...args: unknown[]) => {
        assert.deepEqual(args, [
          "thread",
          "agent",
          undefined,
          "acknowledgement",
        ]);
        return {
          history,
          online: true,
          decide: (id: string, choice: string) => {
            assert.equal(id, "acknowledgement");
            assert.equal(choice, "allow");
            return decision.promise;
          },
          openConversation: async () => {},
        };
      },
    },
    "../../lib/api/pluginSkillsApi": { pluginSkillsApi: {} },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
    "../../theme/designTokens": { fonts: {} },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./authoredSkillReviewState": {
      AUTHORED_SKILL_ACTIONS: [{ action: "allow", label: "Allow publication" }],
      authoredReviewMatches: () => true,
      supportedAuthoredSkillActions: () => ["allow"],
      authoredSkillActionRequest: () => ({ kind: "decision", choice: "allow" }),
    },
  });
  const exports = harness.load(
    fileURLToPath(new URL("./NativeAuthoredSkillReview.tsx", import.meta.url)),
  ) as Record<string, (props: any) => Element>;
  harness.mount(() => {
    if (kind === "launcher")
      return exports.NativeAuthoredSkillReview!({
        acknowledgement,
        conversationId: "thread",
      });
    if (kind === "page")
      return exports.AuthoredSkillPage!({
        route: { name: "AuthoredSkillReview", params },
        navigation,
      });
    return exports.AuthoredSkillReviewContent!({
      acknowledgement,
      onDecision: () => decision.promise,
    });
  });
  return {
    harness,
    decision,
    pushed,
    history,
    guard: () => {
      harness.flush();
      return guard;
    },
    button: (label: string) =>
      find(harness.flush(), (row) => row.props.label === label),
    content: () =>
      find(
        harness.flush(),
        (row) => row.type === exports.AuthoredSkillReviewContent,
      ),
  };
}

test("the in-chat skill launcher pushes the exact serializable review route", () => {
  const env = setup("launcher");
  env.button("Review skill draft").props.onPress();
  assert.deepEqual(env.pushed, [{ name: "AuthoredSkillReview", params }]);
  env.harness.unmount();
});
test("owner review shows readable files and NyxBot guidance, with no inline edit route", () => {
  const env = setup();
  assert.throws(
    () => env.button("Edit before publishing"),
    /Element not found/,
  );
  const guidance = find(
    env.harness.flush(),
    (row) => row.props.children === "To change this skill, ask NyxBot.",
  );
  assert.ok(guidance);
  assert.deepEqual(env.pushed, []);
  assert.equal(env.guard().defaultRole, "back");
  assert.equal(env.guard().discardOnClose, undefined);
  env.harness.unmount();
});
test("publication guards Back and navigation until decision and refresh settle", async () => {
  const env = setup();
  env.button("Allow publication").props.onPress();
  assert.equal(env.guard().disabled, true);
  env.decision.resolve(undefined);
  await env.harness.settle();
  assert.equal(env.guard().disabled, false);
  env.harness.unmount();
});
test("review page resolves its acknowledgement from authoritative history and keeps the existing decision path", async () => {
  const env = setup("page");
  assert.equal(env.content().props.acknowledgement, acknowledgement);
  const operation = env.content().props.onDecision("allow");
  env.decision.resolve(undefined);
  await operation;
  env.history.acknowledgements = [];
  env.harness.render();
  assert.throws(() => env.content(), /Element not found/);
  env.harness.unmount();
});
