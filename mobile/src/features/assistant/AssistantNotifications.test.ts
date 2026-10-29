import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness, deferred } from "./assistantHookTestHarness";
import { createConnectionState } from "../../lib/api/connectionState";
import { advanceAuthEpoch } from "../../lib/auth/authEpoch";
import { sanitizeAssistantMessageContent } from "./assistantChatContent";
import { groupPendingActionSchema } from "../groups/groupContracts";
type E = { type: unknown; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const text = (n: any): string =>
  typeof n === "string" || typeof n === "number"
    ? String(n)
    : n?.props
      ? [n.props.children].flat(Infinity).map(text).join(" ")
      : "";
function render(
  file: string,
  name: string,
  props: any,
  mocks: Record<string, any> = {},
) {
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      AppState: { currentState: "active" },
    },
    "lucide-react-native": {
      X: "X",
      Info: "Info",
      TriangleAlert: "Alert",
      ArrowDown: "Arrow",
      Wifi: "Wifi",
      WifiOff: "WifiOff",
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: {
          bg: "#09090b",
          textMuted: "tertiary",
          dangerTone: {},
          warningTone: {},
          successTone: {},
        },
      }),
    },
    "../../components/workspace/WorkspaceUI": {
      Card: "Card",
      Label: "Label",
      ActionButton: "Button",
    },
    ...mocks,
  });
  const C = h.load(fileURLToPath(new URL(file, import.meta.url)))[name] as (
    p: any,
  ) => E;
  h.mount(() => C(props));
  return { h, nodes: () => nodes(h.value()), text: () => text(h.value()) };
}
test("request error dismissal hides only that message; jump action has a 44pt accessible target", () => {
  const props = { message: "First failure" };
  const x = render("./AssistantBanner.tsx", "AssistantErrorNotice", props, {
    "../../hooks/useNetworkStatus": {},
    "@react-navigation/native": {},
    "../../lib/api/connectionState": {},
  });
  assert.match(x.text(), /First failure/);
  x.h.value().props.onDismiss();
  x.h.flush();
  assert.equal(x.h.value(), null);
  props.message = "Different failure";
  x.h.render();
  assert.match(x.text(), /Different failure/);
  x.h.unmount();
  let jumps = 0;
  const p = { visible: false, onPress: () => jumps++ };
  const y = render("./AssistantBanner.tsx", "JumpToLatest", p, {
    "../../hooks/useNetworkStatus": {},
    "@react-navigation/native": {},
    "../../lib/api/connectionState": {},
  });
  assert.equal(y.h.value(), null);
  p.visible = true;
  y.h.render();
  const control = y
    .nodes()
    .find((n) => n.props.accessibilityLabel === "Jump to latest message")!;
  assert.equal(control.props.style.width, 32);
  assert.equal(control.props.style.height, 32);
  assert.equal(control.props.style.borderRadius, 16);
  assert.equal(control.props.style.width + control.props.hitSlop * 2, 44);
  assert.equal(control.props.style.backgroundColor, "#09090bf2");
  assert.equal(control.props.style.bottom, 12);
  assert.equal(
    y.h.value().props.style.height,
    56,
    "positive accessibility bounds contain the button and its expanded hit area",
  );
  assert.equal(y.h.value().props.style.position, "absolute");
  assert.equal(y.h.value().props.style.bottom, 0);
  assert.equal(y.text(), "", "icon only");
  assert.ok(
    y
      .nodes()
      .some(
        (n) =>
          n.type === "Arrow" &&
          n.props.size === 16 &&
          n.props.color === "tertiary",
      ),
  );
  control.props.onPress();
  assert.equal(jumps, 1);
  y.h.unmount();
});
test("connection observation ignores cancellation, stale account and older failures, and restores on any server response", async () => {
  const state = createConnectionState();
  let changes = 0;
  const release = state.subscribe(() => changes++);
  await assert.rejects(
    state.observe(async () => {
      throw new TypeError("Network request failed");
    }),
  );
  assert.equal(state.snapshot(), true);
  await state.observe(async () => ({ status: 403 }));
  assert.equal(state.snapshot(), false);
  const older = deferred<void>();
  const running = state.observe(() => older.promise);
  await state.observe(async () => {});
  older.reject(new Error("old"));
  await assert.rejects(running);
  assert.equal(state.snapshot(), false);
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(
    state.observe(async () => {
      throw new Error("cancelled");
    }, controller.signal),
  );
  assert.equal(state.snapshot(), false);
  const oldAccount = deferred<void>();
  const old = state.observe(() => oldAccount.promise);
  advanceAuthEpoch();
  oldAccount.reject(new Error("old account"));
  await assert.rejects(old);
  assert.equal(state.snapshot(), false);
  assert.equal(changes, 2);
  release();
});
test("connection banner shows exact offline/unreachable/reconnected copy and refreshes on recovery", () => {
  let online = false,
    lost = false,
    listener = () => {};
  let refreshes = 0;
  const x = render(
    "./AssistantBanner.tsx",
    "AssistantConnectionBanner",
    { onRetry: () => refreshes++ },
    {
      "@react-navigation/native": { useIsFocused: () => true },
      "../../hooks/useNetworkStatus": {
        useNetworkStatus: () => ({ isConnected: online }),
      },
      "../../lib/api/connectionState": {
        assistantConnection: {
          snapshot: () => lost,
          subscribe: (fn: () => void) => {
            listener = fn;
            return () => {};
          },
        },
      },
    },
  );
  assert.match(x.text(), /You're offline. NyxBot can't send, run or refresh/);
  online = true;
  lost = true;
  listener();
  x.h.render();
  assert.match(x.text(), /Can't reach NyxID/);
  lost = false;
  listener();
  x.h.flush();
  assert.match(x.text(), /Back online/);
  assert.equal(refreshes, 1);
  x.h.unmount();
});
test("private idle refresh shares metadata stream, coalesces matching events and fences switched threads", async () => {
  let event: (e: any) => void = () => {};
  const calls: string[] = [];
  const props = { active: true, id: "chat", paused: false };
  const h = hookHarness({
    "../groups/useGroupsLive": {
      useGroupsLive: (_e: any, _a: any, _available: any, callback: any) => {
        event = callback;
        return true;
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistantLiveRefresh.ts", import.meta.url)),
  ).useAssistantLiveRefresh as any;
  h.mount(() =>
    hook(props.active, props.id, props.paused, async (id: string) => {
      calls.push(id);
    }),
  );
  event({ type: "conversation", id: "other" });
  event({ type: "conversation", id: "chat" });
  event({ type: "conversation", id: "chat" });
  await new Promise((r) => setTimeout(r, 180));
  await h.settle();
  assert.deepEqual(calls, ["chat"]);
  event({ type: "resync" });
  props.id = "next";
  h.render();
  await new Promise((r) => setTimeout(r, 180));
  assert.equal(calls.length, 1);
  props.paused = true;
  h.render();
  event({ type: "conversation", id: "next" });
  await new Promise((r) => setTimeout(r, 180));
  assert.equal(calls.length, 1);
  h.unmount();
});
test("assistant text strips complete and streaming XML/DSML tool payloads with exact web rules", () => {
  for (const [raw, expected] of [
    ["Hi<function_calls>private</function_calls>\nDone", "Hi\n\nDone"],
    ["Hi\n<function_calls>private", "Hi"],
    ["A<｜DSML｜function_calls>private</｜DSML｜function_calls>B", "A\nB"],
    ["A<|DSML|function_calls>private", "A"],
    ["user <b>text</b>", "user <b>text</b>"],
  ])
    assert.equal(sanitizeAssistantMessageContent(raw!), expected);
});
const person = "11111111-1111-4111-8111-111111111111";
const action = groupPendingActionSchema.parse({
  conversation_id: `nyxa-${"a".repeat(32)}`,
  summary: "Deploy",
  expires_at: "2099-01-01T00:00:00Z",
  agent_id: "agent",
  acknowledgement_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  confirm_phrase: "yes 1234",
});
test("real crew pending card follows phrase/person/review/expiry gates and web action copy", () => {
  const choices: any[] = [];
  const props: any = {
    action,
    name: "Release tester",
    actorId: person,
    organization: false,
    blocked: false,
    onAnswer: (yes: boolean) => choices.push(yes),
    onDecision: (c: string) => choices.push(c),
    onReview: () => choices.push("review"),
  };
  const x = render("../groups/GroupPendingCard.tsx", "GroupPendingCard", props);
  const press = (label: string) => {
    const n = x.nodes().find((n) => n.props.label === label);
    assert.ok(n, label);
    n.props.onPress();
  };
  assert.ok(
    x.nodes().some((n) => n.props.accessibilityLabel === "Confirm: Deploy"),
  );
  press("Confirm");
  press("Cancel");
  assert.deepEqual(choices, [true, false]);
  props.action = {
    ...action,
    triggering_person: { id: person, display_name: "You" },
    can_decide: true,
  };
  x.h.render();
  press("Allow");
  press("Deny");
  assert.deepEqual(choices.slice(-2), ["allow", "deny"]);
  props.actorId = "other";
  x.h.render();
  assert.ok(!x.nodes().some((n) => n.props.label === "Allow"));
  assert.match(x.text(), /Awaiting.*You/);
  props.actorId = person;
  props.action = {
    ...props.action,
    authored_skill: { agent_id: "agent", proposal_id: "proposal" },
  };
  x.h.render();
  press("Review draft");
  assert.equal(choices.at(-1), "review");
  props.action = { ...action, expires_at: "2020-01-01T00:00:00Z" };
  x.h.render();
  assert.match(x.text(), /request expired/);
  assert.ok(!x.nodes().some((n) => n.type === "Button"));
  x.h.unmount();
});
