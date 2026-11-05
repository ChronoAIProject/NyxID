import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";

type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
function setup() {
  let epoch = 7,
    active = true,
    resets = 0,
    refreshed = 0,
    completed = 0;
  const busy: boolean[] = [];
  const result = deferred<void>();
  const data = {
    active_turn_id: null as string | null,
    pending_actions: [],
    binding_state: "bound",
    agent_id: "agent",
  };
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "@tanstack/react-query": {
      useQuery: () => ({
        data,
        refetch: async () => {
          refreshed++;
        },
      }),
    },
    "../../components/workspace/WorkspaceUI": {
      Card: "Card",
      Label: "Label",
      ActionButton: "ActionButton",
    },
    "../../lib/api/assistantApi": {
      assistantApi: {
        resetContext: () => {
          resets++;
          return result.promise;
        },
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
  });
  const C = h.load(
    fileURLToPath(new URL("./AssistantWorkingContext.tsx", import.meta.url)),
  ).AssistantWorkingContext as any;
  const onBusyChange = (value: boolean) => busy.push(value);
  h.mount(() =>
    C({
      conversationId: "nyxa-thread",
      active,
      onReset: () => completed++,
      onManageMemory: () => {},
      onBusyChange,
    }),
  );
  const button = (label: string) =>
    nodes(h.value()).find(
      (n) => n.type === "ActionButton" && n.props.label === label,
    )!;
  return {
    h,
    result,
    busy,
    data,
    button,
    counts: () => ({ resets, refreshed, completed }),
    account: () => {
      epoch++;
      h.render();
    },
    background: () => {
      active = false;
      h.render();
    },
  };
}

test("context reset publishes pending state to the page guard, is single-flight, and releases on completion", async () => {
  const x = setup();
  x.button("Reset context").props.onPress();
  x.h.flush();
  const confirm = x.button("Confirm reset");
  confirm.props.onPress();
  confirm.props.onPress();
  x.h.flush();
  assert.equal(x.counts().resets, 1);
  assert.equal(x.busy.at(-1), true);
  assert.equal(x.button("Confirm reset").props.disabled, true);
  x.result.resolve();
  await x.h.settle();
  assert.deepEqual(x.counts(), { resets: 1, refreshed: 1, completed: 1 });
  assert.equal(x.busy.at(-1), false);
  x.h.unmount();
});

test("context reset does not refresh or signal another account/background/unmounted page", async () => {
  for (const change of ["account", "background", "unmount"] as const) {
    const x = setup();
    x.button("Reset context").props.onPress();
    x.h.flush();
    x.button("Confirm reset").props.onPress();
    x.h.flush();
    if (change === "unmount") x.h.unmount();
    else x[change]();
    x.result.resolve();
    await x.h.settle();
    assert.deepEqual(x.counts(), { resets: 1, refreshed: 0, completed: 0 });
    assert.equal(x.busy.at(-1), false);
    if (change !== "unmount") x.h.unmount();
  }
});
