import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness, deferred } from "./assistantHookTestHarness";
import { historySchema, messageSchema } from "./assistantTypes";
import { describeEventNotice, eventNotices } from "./assistantEventNotices";
import { advanceAuthEpoch } from "../../lib/auth/authEpoch";

type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const text = (n: any): string =>
  typeof n === "string" || typeof n === "number"
    ? String(n)
    : n?.props
      ? [n.props.children].flat(Infinity).map(text).join(" ")
      : "";
const approval = {
  id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  service_name: "GitHub",
  service_slug: "github",
  summary: "Create the issue",
  agent_key_prefix: "nyx_key_123",
  approval_mode: "per_request",
  expires_at: "2099-01-01T00:00:00Z",
};
function render(file: string, name: string, props: any) {
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": { View: "View", Pressable: "Pressable" },
    "lucide-react-native": new Proxy({}, { get: (_, key) => String(key) }),
    "../../components/AgentAvatar": { AgentAvatar: "Avatar" },
    "./AssistantMessageFooter": { AssistantMessageTime: "Time" },
    "../../components/workspace/WorkspaceUI": {
      Card: "Card",
      Label: "Label",
      ActionButton: "Button",
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: { successTone: {}, warningTone: {}, dangerTone: {} },
      }),
    },
  });
  const C = h.load(fileURLToPath(new URL(file, import.meta.url)))[name] as any;
  h.mount(() => C(props));
  return {
    h,
    nodes: () => nodes(h.value()),
    text: () => text(h.value()),
    button: (label: string) =>
      nodes(h.value()).find((n) => n.props.label === label)!,
  };
}
test("real proxy approval shows scope, commits once, recovers failures and refuses stale/expired controls", async () => {
  const request = deferred<void>();
  let count = 0;
  const props = {
    approval,
    disabled: false,
    onDecide: async () => {
      count++;
      await request.promise;
    },
  };
  const x = render(
    "./AssistantPendingState.tsx",
    "AssistantApprovalCard",
    props,
  );
  assert.match(x.text(), /Approval required.*Review the destination/);
  assert.match(x.text(), /Scope.*github.*nyx_key_123.*per-request approval/);
  const decide = x.button("Approve and send").props.onPress;
  decide();
  decide();
  x.h.flush();
  assert.equal(count, 1);
  assert.equal(x.button("Deny").props.disabled, true);
  request.reject(new Error("Connection lost"));
  await x.h.settle();
  assert.match(x.text(), /Could not send your decision/);
  assert.equal(x.button("Deny").props.disabled, false);
  props.onDecide = async () => {
    count++;
  };
  x.h.render();
  x.button("Deny").props.onPress();
  await x.h.settle();
  assert.match(x.text(), /Request denied.*Nothing was sent/);
  assert.equal(x.button("Approve and send"), undefined);
  x.h.unmount();
  const p = { ...props, approval: { ...approval, expires_at: "2000-01-01" } };
  const expired = render(
    "./AssistantPendingState.tsx",
    "AssistantApprovalCard",
    p,
  );
  expired.button("Approve and send").props.onPress();
  assert.equal(count, 2);
  assert.equal(expired.button("Approve and send").props.disabled, true);
  expired.h.unmount();
});
test("external wait preserves expiry/auto-resume and pending update wording from real history", () => {
  const x = render("./AssistantPendingState.tsx", "AssistantWaitingNote", {
    items: [
      {
        kind: "service",
        since: "2026-10-28",
        title: "Connect GitHub",
        detail: "Finish connecting your account.",
        expires_at: "2099-01-01T00:00:00Z",
      },
    ],
    agentName: "Researcher",
    pendingEvents: 2,
  });
  assert.match(
    x.text(),
    /2\s+updates\s+waiting for\s+Researcher\s*'s next turn/,
  );
  assert.match(x.text(), /Connect GitHub.*expires.*Finish connecting/);
  assert.match(
    x.text(),
    /Researcher\s+continues here by itself when this happens; no need to reply/,
  );
  assert.ok(
    x
      .nodes()
      .some(
        (n) =>
          n.props.accessibilityLabel === "Waiting" &&
          n.props.accessibilityLiveRegion === "polite",
      ),
  );
  x.h.unmount();
});
test("server-authored events use exact web summaries, multiline excerpts and completion/failure tones", () => {
  const notices = eventNotices(
    'NyxID events (authored by NyxID):\n- Specialist researcher replied. Reply excerpt: "First line\n- quoted bullet"\n- The user finished connecting github (connect_link_id abc).',
  );
  assert.equal(notices.length, 2);
  assert.deepEqual(describeEventNotice(notices[0]!), {
    kind: "reply",
    tone: "neutral",
    summary: "researcher replied",
    detail: "First line\n- quoted bullet",
    agent: "researcher",
  });
  assert.equal(
    describeEventNotice(notices[1]!).summary,
    "Finished connecting github",
  );
  assert.equal(
    describeEventNotice(
      'Specialist researcher failed (timeout). Reply excerpt: "Try later"',
    ).tone,
    "destructive",
  );
  assert.equal(
    describeEventNotice(
      "The user rejected the github approval request (Create issue) via mobile.",
    ).summary,
    "You rejected the github request via Mobile",
  );
  const item = messageSchema.parse({
    id: "event",
    seq: 1,
    turn_id: "t",
    role: "event",
    text: "Run the review",
    status: "completed",
    error_code: null,
    created_at: "2026-10-28",
    origin: "trigger",
    automation: { trigger_id: "trigger", run_id: "run", label: "Daily review" },
  });
  const x = render("./AssistantSystemMessage.tsx", "AssistantSystemMessage", {
    message: item,
    onOpenAutomations: () => {},
  });
  assert.equal(
    x.nodes().find((n) => n.props.notice)?.props.notice.summary,
    "Automation · Daily review",
  );
  x.h.unmount();
  const y = render("./AssistantSystemMessage.tsx", "AssistantSystemMessage", {
    message: { ...item, role: "orchestrator" },
    onOpenAutomations: () => {},
  });
  assert.equal(y.h.value().props.accessibilityLabel, "Message from NyxBot");
  assert.match(y.text(), /From NyxBot.*Run the review/);
  y.h.unmount();
});
test("proxy approval API reuses the native decision endpoint contract and rejects mismatched identities", async () => {
  const calls: any[] = [];
  let mismatch = false;
  const h = hookHarness({
    "./http": {
      submitChallengeDecisionRequest: async (...args: any[]) => {
        calls.push(args);
        return {
          challenge_id: mismatch ? "other" : approval.id,
          status: "APPROVED",
        };
      },
    },
  });
  const api = h.load(
    fileURLToPath(new URL("../../lib/api/assistantApi.ts", import.meta.url)),
  ).assistantApi as any;
  await api.decideApproval(approval.id, true, "decision-key");
  assert.deepEqual(calls[0], [
    approval.id,
    "APPROVE",
    undefined,
    "decision-key",
  ]);
  mismatch = true;
  await assert.rejects(
    api.decideApproval(approval.id, false, "denial-key"),
    /mismatch/,
  );
});
test("real hook fences duplicate, expired, offline, account and thread approval completions", async () => {
  const A = `nyxa-${"a".repeat(32)}`,
    B = `nyxa-${"b".repeat(32)}`;
  const page = (id: string) =>
    historySchema.parse({
      conversation: {
        id,
        title: "Review",
        model: "nyxagent/chat",
        created_at: "2026-10-28",
        last_message_at: "2026-10-28",
        message_count: 0,
        active_turn: null,
        pending_events: 2,
      },
      messages: [],
      approvals: [approval],
      before_seq: null,
    });
  let calls = 0,
    reads = 0;
  let network: any;
  const response = deferred<void>();
  const h = hookHarness<any>({
    "./useAssistantLiveRefresh": { useAssistantLiveRefresh: () => {} },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove() {} }),
      },
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@react-native-community/netinfo": {
      addEventListener: (cb: any) => {
        network = cb;
        return () => {};
      },
    },
    "@tanstack/react-query": { useQuery: () => ({ data: { agents: [] } }) },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "../../lib/api/assistantApi": {
      assistantApi: {
        history: async (id: string) => {
          reads++;
          return page(id);
        },
        list: async () => ({ conversations: [], next_cursor: null }),
        models: async () => [],
        decideApproval: async () => {
          calls++;
          await response.promise;
        },
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
  ).useAssistant as any;
  h.mount(() => hook(A));
  await h.settle();
  h.value().history.approvals[0].expires_at = "2000-01-01";
  await assert.rejects(
    h.value().decideApproval(approval.id, true),
    /no longer available/,
  );
  assert.equal(calls, 0);
  h.value().history.approvals[0].expires_at = approval.expires_at;
  network({ isConnected: false });
  h.flush();
  await assert.rejects(
    h.value().decideApproval(approval.id, true),
    /Reconnect/,
  );
  assert.equal(calls, 0);
  network({ isConnected: true });
  await h.settle();
  const promise = h.value().decideApproval(approval.id, true);
  await h.value().decideApproval(approval.id, true);
  assert.equal(calls, 1);
  h.value().openConversation(B);
  await h.settle();
  const before = reads;
  response.resolve();
  await promise;
  await h.settle();
  assert.equal(
    reads,
    before,
    "stale completion never refreshes the new conversation",
  );
  assert.equal(h.value().conversationId, B);
  advanceAuthEpoch();
  await assert.rejects(
    h.value().decideApproval(approval.id, true),
    /Reconnect/,
  );
  assert.equal(calls, 1);
  h.unmount();
});

test("live metadata refresh projects authoritative approval, connected and failed states once per burst", async () => {
  const id = `nyxa-${"c".repeat(32)}`;
  for (const kind of ["approval", "connected", "failed"] as const) {
    let changed = false;
    let reads = 0;
    let event: (value: any) => void = () => {};
    let liveRefresh: (...args: any[]) => unknown;
    const page = () =>
      historySchema.parse({
        conversation: {
          id,
          title: "Live review",
          model: "nyxagent/chat",
          created_at: "2026-10-28",
          last_message_at: "2026-10-28",
          message_count: changed && kind !== "approval" ? 1 : 0,
          active_turn: null,
        },
        messages:
          !changed || kind === "approval"
            ? []
            : [
                {
                  id: `message-${kind}`,
                  seq: 1,
                  turn_id: "turn",
                  role: kind === "connected" ? "event" : "assistant",
                  text:
                    kind === "connected"
                      ? "NyxID events (authored by NyxID):\n- The user finished connecting github (connect_link_id abc)."
                      : "",
                  status: kind === "failed" ? "failed" : "completed",
                  error_code: kind === "failed" ? "model_interrupted" : null,
                  created_at: "2026-10-28",
                },
              ],
        approvals: changed && kind === "approval" ? [approval] : [],
        before_seq: null,
      });
    const h = hookHarness<any>({
      "./useAssistantLiveRefresh": {
        useAssistantLiveRefresh: (...args: any[]) => liveRefresh(...args),
      },
      "../groups/useGroupsLive": {
        useGroupsLive: (
          _epoch: number,
          _active: boolean,
          _available: any,
          listener: any,
        ) => {
          event = listener;
          return true;
        },
      },
      "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
      "react-native": {
        AppState: {
          currentState: "active",
          addEventListener: () => ({ remove() {} }),
        },
      },
      "@react-navigation/native": { useIsFocused: () => true },
      "@react-native-community/netinfo": { addEventListener: () => () => {} },
      "@tanstack/react-query": { useQuery: () => ({ data: { agents: [] } }) },
      "../../lib/api/agentsApi": { agentsApi: {} },
      "../../lib/api/assistantApi": {
        assistantApi: {
          history: async () => {
            reads++;
            return page();
          },
          list: async () => ({ conversations: [], next_cursor: null }),
          models: async () => [],
        },
      },
    });
    liveRefresh = h.load(
      fileURLToPath(new URL("./useAssistantLiveRefresh.ts", import.meta.url)),
    ).useAssistantLiveRefresh as any;
    const hook = h.load(
      fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
    ).useAssistant as any;
    h.mount(() => hook(id));
    await h.settle();
    const baseline = reads;
    changed = true;
    event({ type: "conversation", id });
    event({ type: "conversation", id });
    await new Promise((resolve) => setTimeout(resolve, 180));
    await h.settle();
    assert.equal(
      reads,
      baseline + 1,
      `${kind}: repeated SSE metadata coalesces into one real history read`,
    );
    const history = h.value().history;
    if (kind === "approval") {
      const x = render("./AssistantPendingState.tsx", "AssistantApprovalCard", {
        approval: history.approvals[0],
        disabled: false,
        onDecide: async () => {},
      });
      assert.match(x.text(), /Approval required/);
      assert.ok(
        x.nodes().some((n) => n.props.accessibilityLiveRegion === "polite"),
      );
      x.h.unmount();
    } else if (kind === "connected") {
      const x = render(
        "./AssistantSystemMessage.tsx",
        "AssistantSystemMessage",
        {
          message: history.messages[0],
          onOpenAutomations: () => {},
        },
      );
      assert.equal(x.h.value().props.accessibilityLiveRegion, "polite");
      assert.equal(
        x.nodes().find((n) => n.props.notice)?.props.notice.summary,
        "Finished connecting github",
      );
      assert.equal(
        history.messages.length,
        1,
        "no duplicate event rows or invented completion toasts",
      );
      x.h.unmount();
    } else {
      assert.equal(history.messages[0].status, "failed");
      assert.equal(history.messages[0].error_code, "model_interrupted");
      assert.equal(h.value().sending, false);
      assert.equal(
        history.messages.length,
        1,
        "one durable failed reply, without duplicate live messages",
      );
    }
    h.unmount();
  }
});
