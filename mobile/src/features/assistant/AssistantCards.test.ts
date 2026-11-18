import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "./assistantHookTestHarness";
import {
  acknowledgementSchema,
  cmaOperationSchema,
  historySchema,
} from "./assistantTypes";
import {
  countDecisions,
  decisionSummary,
  decisionTitle,
  groupDecisions,
} from "./assistantDecisions";
import { cmaPresentation } from "./assistantCma";

type Element = { type: unknown; props: Record<string, any> };
const elements = (node: any): Element[] =>
  !node?.props
    ? []
    : [node, ...[node.props.children].flat(Infinity).flatMap(elements)];
const words = (node: any): string =>
  typeof node === "string" || typeof node === "number"
    ? String(node)
    : [node?.props?.children]
        .flat(Infinity)
        .map((n) =>
          n?.props
            ? words(n)
            : typeof n === "string" || typeof n === "number"
              ? String(n)
              : "",
        )
        .join(" ");
function render(file: string, name: string, props: any) {
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      View: "View",
      StyleSheet: { create: (value: unknown) => value },
    },
    "../../components/workspace/WorkspaceUI": {
      Card: "Card",
      Label: "Label",
      ActionButton: "ActionButton",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const C = h.load(fileURLToPath(new URL(file, import.meta.url)))[name] as (
    props: any,
  ) => Element;
  h.mount(() => C(props));
  return { h, nodes: () => elements(h.value()), text: () => words(h.value()) };
}
const at = "2026-10-28T00:00:00Z";
const ack = acknowledgementSchema.parse({
  id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  kind: "machine",
  status: "pending",
  summary: "Allow Commands",
  created_at: at,
  expires_at: "2099-01-01T00:00:00Z",
  service_name: null,
  service_slug: null,
  tool_name: "machine",
  confirmation: { risk: "sensitive", reason: "policy" },
  creation_turn_id: "turn",
  activity_id: "tool",
});
const operation = cmaOperationSchema.parse({
  id: "cma-1",
  turn_id: "turn",
  kind: "cma_response",
  state: "waiting",
  status: "queued",
  phase: null,
  reason: null,
  operation_id: "op-1",
  agent_id: "agent-1",
  idempotency_key: "key-1",
  monitor_name: "Researcher",
  created_at: at,
  checked_at: at,
});

test("real acknowledgement card uses web risk/help copy and existing guarded decision callback", () => {
  const choices: string[] = [];
  const props = {
    row: ack,
    selected: true,
    disabled: false,
    deciding: false,
    onDecision: (choice: string) => choices.push(choice),
  };
  const x = render(
    "./AssistantAcknowledgementCard.tsx",
    "AssistantAcknowledgementCard",
    props,
  );
  assert.match(x.text(), /Confirm: Allow Commands/);
  assert.match(
    x.text(),
    /This machine's or saved login's confirmation setting asks/,
  );
  assert.match(x.text(), /This permission applies to this chat only/);
  x.nodes()
    .find((node) => node.props.label === "Allow")!
    .props.onPress();
  assert.deepEqual(choices, ["allow"]);
  props.disabled = true;
  x.h.render();
  assert.ok(
    x
      .nodes()
      .filter((node) => node.type === "ActionButton")
      .every((node) => node.props.disabled),
  );
  props.row = { ...ack, status: "used" };
  x.h.render();
  assert.equal(x.h.value(), null);
  x.h.unmount();
});

test("automatic machine and saved-login receipts parse without claiming the operation succeeded", () => {
  const automatic = acknowledgementSchema.parse({
    ...ack,
    kind: "saved_login",
    status: "used",
    decided_by: "access_mode",
    link: "/assistant?panel=settings",
    decided_at: at,
  });
  assert.match(
    decisionTitle(automatic),
    /Allowed automatically by Full access · bypassed policy confirmation/,
  );
  assert.deepEqual(
    decisionSummary(
      countDecisions([
        automatic,
        { ...ack, status: "denied", decided_by: "user" },
      ]),
    ),
    ["1 denied", "1 allowed by Full access"],
  );
  assert.equal(groupDecisions([automatic, automatic])[0]?.count, 2);
});

test("CMA presentation matches queued, approval, failure, unknown submission, stopped tracking and completed-result states", () => {
  for (const [patch, label] of [
    [{}, "Queued"],
    [{ status: "waiting_approval" }, "Needs approval"],
    [{ status: "connection_required" }, "Needs connection"],
    [{ status: "failed" }, "Failed"],
    [
      { state: "settled", status: null, reason: "submission_uncertain" },
      "Submission unconfirmed",
    ],
    [{ state: "cancelled" }, "Tracking stopped"],
    [
      { state: "settled", status: "completed", reason: "result_expired" },
      "Completed",
    ],
  ] as const)
    assert.equal(cmaPresentation({ ...operation, ...patch }).label, label);
  const x = render("./AssistantCmaOperation.tsx", "AssistantCmaOperation", {
    operation: { ...operation, state: "cancelled" },
  });
  assert.match(x.text(), /Tracking ended. CMA may still be running/);
  assert.match(x.text(), /Idempotency-Key/);
  assert.ok(
    x.nodes().some((node) => node.props.accessibilityLabel === "CMA progress"),
  );
  x.h.unmount();
});

test("authoritative history retains CMA and proxy-approval scope without inventing blocks", () => {
  const history = historySchema.parse({
    conversation: {
      id: `nyxa-${"a".repeat(32)}`,
      title: "T",
      model: "M",
      created_at: at,
      last_message_at: at,
      message_count: 0,
      active_turn: null,
    },
    messages: [],
    before_seq: null,
    cma_operations: [operation],
    approvals: [
      {
        id: "approval",
        service_name: "GitHub",
        summary: "Post issue",
        expires_at: at,
        service_slug: "api-github",
        approval_mode: "per_request",
        agent_key_prefix: "nyx_key_",
        created_at: at,
      },
    ],
  });
  assert.equal(history.cma_operations?.[0]?.operation_id, "op-1");
  assert.equal(history.approvals[0]?.approval_mode, "per_request");
});
