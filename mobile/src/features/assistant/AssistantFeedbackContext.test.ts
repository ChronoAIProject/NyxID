import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness, deferred } from "./assistantHookTestHarness";
import { assistantContextSchema } from "./assistantTypes";

type Element = { type: unknown; props: Record<string, any> };
function nodes(n: any): Element[] {
  return n?.props
    ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)]
    : [];
}
function words(n: any): string {
  return typeof n === "string" || typeof n === "number"
    ? String(n)
    : [n?.props?.children]
        .flat(Infinity)
        .map((x) => (x?.props ? words(x) : typeof x === "string" ? x : ""))
        .join(" ");
}
function setup(
  file: string,
  name: string,
  props: any,
  mocks: Record<string, any> = {},
) {
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Modal: "Modal",
      Pressable: "Pressable",
      ScrollView: "ScrollView",
    },
    "lucide-react-native": { ThumbsUp: "Up", ThumbsDown: "Down" },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "Button",
      Card: "Card",
      Field: "Field",
      Label: "Label",
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: {
          primaryTone: { bg: "purple" },
          successTone: { text: "green" },
          dangerTone: { text: "red" },
        },
      }),
    },
    ...mocks,
  });
  const C = h.load(fileURLToPath(new URL(file, import.meta.url)))[name] as (
    p: any,
  ) => Element;
  h.mount(() => C(props));
  const find = (label: string) => {
    const n = nodes(h.value()).find(
      (n) => n.props.label === label || n.props.accessibilityLabel === label,
    );
    assert.ok(n, label);
    return n;
  };
  return {
    h,
    find,
    press: (label: string) => {
      find(label).props.onPress();
      h.flush();
    },
    text: () => words(h.value()),
  };
}
const id = `nyxa-${"a".repeat(32)}`;
const context = assistantContextSchema.parse({
  conversation_id: id,
  agent_id: "agent",
  binding_state: "bound",
  pending_actions: ["Search"],
  checkpoint: {
    turn_id: "t",
    status: "completed",
    goal: "Find evidence",
    updated_at: "2026-10-28T00:00:00Z",
    completed_actions: [{ label: "Read", turn_id: "t" }],
  },
});
test("real working context renders checkpoint, confirms reset once and opens memory", async () => {
  let resets = 0,
    refreshes = 0,
    notifications = 0;
  const memories: string[] = [];
  const wait = deferred<typeof context>();
  let options: any;
  const x = setup(
    "./AssistantWorkingContext.tsx",
    "AssistantWorkingContext",
    {
      conversationId: id,
      active: true,
      onReset: () => notifications++,
      onManageMemory: (id: string) => memories.push(id),
    },
    {
      "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
      "../../lib/api/assistantApi": {
        assistantApi: {
          resetContext: () => {
            resets++;
            return wait.promise;
          },
        },
      },
      "@tanstack/react-query": {
        useQuery: (o: any) => {
          options = o;
          return {
            data: context,
            isPending: false,
            isError: false,
            refetch: async () => refreshes++,
          };
        },
      },
    },
  );
  assert.match(x.text(), /Goal:.*Find evidence/);
  assert.match(x.text(), /Completed since then:.*Read/);
  assert.equal(
    options.refetchInterval({ state: { data: { active_turn_id: "t" } } }),
    3000,
  );
  x.press("Manage memory notes");
  assert.deepEqual(memories, ["agent"]);
  x.press("Reset context");
  assert.match(x.text(), /transcript stays/);
  assert.equal(resets, 0);
  x.press("Cancel");
  x.press("Reset context");
  x.press("Confirm reset");
  x.press("Confirm reset");
  assert.equal(resets, 1);
  wait.resolve(context);
  await x.h.settle();
  assert.equal(refreshes, 1);
  assert.equal(notifications, 1);
  x.h.unmount();
});
test("working context disables active-turn reset and fences stale account reset completion", async () => {
  let epoch = 1,
    done = 0;
  const wait = deferred<typeof context>();
  const props = {
    conversationId: id,
    active: true,
    onReset: () => done++,
    onManageMemory: () => {},
  };
  const query: any = {
    data: { ...context, active_turn_id: "running" },
    isPending: false,
    isError: false,
    refetch: async () => {},
  };
  const x = setup(
    "./AssistantWorkingContext.tsx",
    "AssistantWorkingContext",
    props,
    {
      "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
      "../../lib/api/assistantApi": {
        assistantApi: { resetContext: () => wait.promise },
      },
      "@tanstack/react-query": { useQuery: () => query },
    },
  );
  assert.equal(x.find("Reset context").props.disabled, true);
  query.data = context;
  x.h.render();
  x.press("Reset context");
  x.press("Confirm reset");
  epoch++;
  wait.resolve(context);
  await x.h.settle();
  assert.equal(done, 0);
  x.h.unmount();
});
test("working context uses existing API routes and validate context identity", async () => {
  const calls: any[] = [];
  let mismatch = false;
  const h = hookHarness({
    "./http": {
      requestJson: async (path: string, options: any) => {
        calls.push({ path, options });
        return {
          ...context,
          conversation_id: mismatch ? `nyxa-${"b".repeat(32)}` : id,
        };
      },
    },
  });
  const api = h.load(
    fileURLToPath(new URL("../../lib/api/assistantApi.ts", import.meta.url)),
  ).assistantApi as any;
  await api.context(id);
  await api.resetContext(id);
  assert.equal(
    calls[0].path,
    `/assistant/nyxagent/conversations/${id}/context`,
  );
  assert.equal(
    calls[1].path,
    `/assistant/nyxagent/conversations/${id}/context/reset`,
  );
  assert.equal(calls[1].options.method, "POST");
  mismatch = true;
  await assert.rejects(api.context(id), /context mismatch/);
});
