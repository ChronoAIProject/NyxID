import { readFileSync } from "node:fs";
import { historySchema } from "./assistantTypes";
import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "./assistantHookTestHarness";
import {
  acknowledgementSchema,
  activitySchema,
  messageSchema,
} from "./assistantTypes";

type Element = { type: unknown; props: Record<string, any> };
function nodes(value: unknown): Element[] {
  if (!value || typeof value !== "object" || !("props" in value)) return [];
  const node = value as Element;
  return [node, ...[node.props.children].flat(Infinity).flatMap(nodes)];
}
function text(value: unknown): string {
  if (typeof value === "string" || typeof value === "number")
    return String(value);
  return value && typeof value === "object" && "props" in value
    ? [(value as Element).props.children].flat(Infinity).map(text).join(" ")
    : "";
}
function setup(file: string, name: string, props: Record<string, any>) {
  const copies: string[] = [];
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      ActivityIndicator: "ActivityIndicator",
      StyleSheet: { create: (value: unknown) => value },
    },
    "lucide-react-native": {
      Check: "Check",
      CircleAlert: "CircleAlert",
      Minus: "Minus",
      ShieldCheck: "ShieldCheck",
      X: "X",
      ChevronDown: "ChevronDown",
      ChevronRight: "ChevronRight",
    },
    "../../components/workspace/FirstLineIcon": { FirstLineIcon: "FirstLineIcon" },
    "../../components/workspace/WorkspaceUI": {
      Label: "Label",
      Notice: "Notice",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "expo-clipboard": {
      setStringAsync: async (value: string) => {
        copies.push(value);
      },
    },
  });
  const Component = h.load(fileURLToPath(new URL(file, import.meta.url)))[
    name
  ] as (props: any) => Element;
  h.mount(() => Component(props));
  return {
    h,
    copies,
    nodes: () => nodes(h.value()),
    text: () => text(h.value()),
    press: (label: string) => {
      const node = nodes(h.value()).find(
        (node) =>
          node.props.accessibilityLabel === label || node.props.label === label,
      );
      assert.ok(node, `Missing ${label}`);
      node.props.onPress();
      h.flush();
    },
  };
}
const at = "2026-10-28T01:00:00Z";
test("real activity disclosure exposes elapsed tool failure copy, truncation and accessible status", () => {
  let toggles = 0;
  const activity = activitySchema.parse({
    id: "tool",
    label: "api-github__get_issue",
    status: "error",
    started_at: at,
    ended_at: "2026-10-28T01:01:03Z",
    failure: { kind: "service_status", status: 503 },
  });
  const x = setup(
    "./AssistantActivityDisclosure.tsx",
    "AssistantActivityDisclosure",
    {
      activities: [activity],
      activityTotal: 12,
      turnLabel: "Turn failed",
      onToggle: () => toggles++,
    },
  );
  assert.match(x.text(), /Turn failed · 12 actions · 1 failed/);
  assert.match(x.text(), /latest 1 of 12 shown/);
  assert.doesNotMatch(x.text(), /HTTP/);
  x.press("Turn details: Turn failed · 12 actions · 1 failed");
  assert.match(x.text(), /1:03/);
  assert.match(x.text(), /The service responded with HTTP 503/);
  assert.match(x.text(), /assistant can continue after a failed step/);
  assert.ok(
    x
      .nodes()
      .some((node) => /Failed, 1:03/.test(node.props.accessibilityLabel ?? "")),
  );
  assert.equal(toggles, 1);
  x.h.unmount();
});
test("a call refused only to ask for confirmation is not counted or shown as a failure", () => {
  const gated = activitySchema.parse({
    id: "tool",
    label: "nyxid__create_schedule",
    status: "error",
    started_at: at,
    ended_at: at,
    failure: { kind: "tool_error" },
  });
  const broken = activitySchema.parse({
    id: "other",
    label: "api-github__get_issue",
    status: "error",
    started_at: at,
    ended_at: at,
    failure: { kind: "service_status", status: 503 },
  });
  const card = acknowledgementSchema.parse({
    id: "d9428888-122b-41e1-b85c-61cd3cbb3210",
    conversation_id: `nyxa-${"a".repeat(32)}`,
    activity_id: "tool",
    kind: "action",
    status: "allowed",
    summary: "Create a scheduled automation",
    service_slug: null,
    service_name: null,
    tool_name: "nyxid__create_schedule",
    created_at: at,
    expires_at: "2099-01-01T00:00:00Z",
  });
  const only = setup(
    "./AssistantActivityDisclosure.tsx",
    "AssistantActivityDisclosure",
    { activities: [gated], decisions: [card], onToggle: () => {} },
  );
  assert.doesNotMatch(only.text(), /failed/);
  const statuses = () =>
    only
      .nodes()
      .map((node) => node.props.status)
      .filter(Boolean)
      .join(",");
  assert.equal(statuses(), "completed");
  only.press(
    only
      .nodes()
      .find((node) =>
        /^Turn details:/.test(node.props.accessibilityLabel ?? ""),
      )!.props.accessibilityLabel,
  );
  assert.match(only.text(), /Asked for your confirmation/);
  assert.doesNotMatch(only.text(), /continue after a failed step/);
  assert.equal(statuses(), "completed,confirmation");
  assert.ok(
    only
      .nodes()
      .some((node) =>
        /Confirmation requested/.test(node.props.accessibilityLabel ?? ""),
      ),
  );
  only.h.unmount();
  const mixed = setup(
    "./AssistantActivityDisclosure.tsx",
    "AssistantActivityDisclosure",
    { activities: [gated, broken], decisions: [card], onToggle: () => {} },
  );
  assert.match(mixed.text(), /1 failed/);
  mixed.h.unmount();
  // While the card is still pending the screen passes no decided rows.
  const pending = setup(
    "./AssistantActivityDisclosure.tsx",
    "AssistantActivityDisclosure",
    {
      activities: [gated],
      confirmationActivityIds: new Set(["tool"]),
      turnLabel: "Working…",
      onToggle: () => {},
    },
  );
  assert.doesNotMatch(pending.text(), /failed/);
  pending.h.unmount();
});
test("real turn-status row announces approval and interruption; queued capacity has the web acknowledgement", () => {
  const props = {
    turn: { turn_id: "t", started_at: at, activities: [] },
    pendingApproval: true,
    interrupted: true,
  };
  const x = setup("./AssistantTurnStatus.tsx", "AssistantTurnStatus", props);
  assert.match(
    x.text(),
    /Waiting for approval\s+Connection interrupted; checking every few seconds/,
  );
  assert.ok(
    x.nodes().some((node) => node.props.accessibilityLiveRegion === "polite"),
  );
  Object.assign(props.turn, { queued: true, waiting_for_capacity: true });
  x.h.render();
  assert.match(x.text(), /Got it — I'm a bit busy right now/);
  assert.match(x.text(), /Queued until another reply finishes/);
  x.h.unmount();
});
test("real failure UI discloses metadata and original request without retrying uncertain effects", async () => {
  const message = messageSchema.parse({
    id: "m",
    seq: 2,
    turn_id: "t",
    role: "assistant",
    text: "Partial",
    status: "failed",
    error_code: "model_interrupted",
    created_at: at,
    error_detail: {
      stage: "stream",
      http_status: 502,
      upstream_request_id: "req",
      response_id: null,
      continuations: 2,
      turn_id: "t",
      occurred_at: at,
    },
    recovery: {
      status: "review",
      reason: "activity_outcome_uncertain",
      text: "Original request",
      attachments: ["file"],
    },
  });
  const x = setup("./AssistantMessageNotices.tsx", "AssistantMessageNotices", {
    message,
    onOpenBilling: () => {},
  });
  assert.match(x.text(), /A tool may have taken effect/);
  assert.doesNotMatch(x.text(), /Original request/);
  const closed = x.nodes().find((node) => node.props.label === "Details")!;
  assert.equal(closed.props.expanded, false, "Details is a disclosure for an in-chat panel");
  x.press("Details");
  assert.match(x.text(), /stage: stream/);
  const open = x.nodes().find((node) => node.props.label === "Hide details")!;
  assert.equal(open.props.expanded, true, "it says it can be closed again");
  assert.ok(
    x.nodes().some((node) => node.props.label === "Usage & credits" && node.props.opens) ||
      !x.nodes().some((node) => node.props.label === "Usage & credits"),
    "Usage & credits, when shown, is marked as leaving the chat",
  );
  x.press("Copy diagnostic ID");
  await x.h.settle();
  assert.equal(x.copies[0], "turn t / request req");
  x.press("Review request");
  assert.match(x.text(), /Original request/);
  x.press("Copy request");
  await x.h.settle();
  assert.equal(x.copies[1], "Original request");
  assert.match(x.text(), /Request copied/);
  x.h.unmount();
});

for (const [state, label, count] of [
  ["waiting", "Waiting for approval", "1 waiting"],
  ["allowed", "Allowed — waiting for execution", "1 allowed"],
  ["completed", "Completed after approval", "1 allowed"],
  ["denied", "Declined", "1 denied"],
  ["expired", "Approval expired", "1 expired"],
  ["cancelled", "Cancelled", "1 cancelled"],
  ["unavailable", "Approval outcome unavailable", "1 outcome unavailable"],
  ["failed", "Failed after approval", "1 allowed"],
]) {
  test(`server decision ${state} renders with truthful counts`, () => {
    const activity = activitySchema.parse({ id: "gated", label: "nyxid__draft_agent_skill", status: "error", started_at: at, ended_at: at,
      failure: { kind: "tool_error" }, decision: { kind: "acknowledgement", id: "card", state } });
    const x = setup("./AssistantActivityDisclosure.tsx", "AssistantActivityDisclosure", { activities: [activity], onToggle: () => {} });
    assert.ok(x.text().includes(count!));
    const summary = `Turn details: Completed · 1 action · ${count}${state === "failed" ? " · 1 failed" : ""}`;
    x.press(summary);
    assert.ok(x.text().includes(label!));
    assert.doesNotMatch(x.text(), /The tool reported an error|continue after a failed step/);
    x.h.unmount();
  });
}

test("2345 completed duplicate and future activity render neutrally, with no success or failure claim", () => {
  for (const [kind, copy] of [
    ["retry_duplicate_action", "Already done in the previous attempt; skipped this time."],
    ["future_kind", "Activity details unavailable in this app version."],
  ]) {
    const activity = activitySchema.parse({ id: "tool", label: "api-lark-2__bitable_record_create", status: "completed", started_at: at, failure: { kind } });
    const x = setup("./AssistantActivityDisclosure.tsx", "AssistantActivityDisclosure", { activities: [activity], onToggle: () => {} });
    x.press("Turn details: Completed · 1 action");
    assert.ok(x.text().includes(copy!));
    const row = x.nodes().find((node) => node.props.accessibilityRole === "text");
    assert.ok(row);
    assert.doesNotMatch(row.props.accessibilityLabel, /Succeeded|Failed|Running/);
    assert.doesNotMatch(x.text(), /failed step|1 failed/);
    x.h.unmount();
  }
});

test("2345 the reported active-turn fragment renders among all 13 actions", () => {
  const wire = JSON.parse(readFileSync(new URL("../../../../frontend/src/lib/assistant/__fixtures__/retry-duplicate-history.json", import.meta.url), "utf8"));
  const page = historySchema.parse(wire);
  const x = setup("./AssistantActivityDisclosure.tsx", "AssistantActivityDisclosure", { activities: page.conversation.active_turn!.activities, onToggle: () => {} });
  x.press("Turn details: Working… · 13 actions");
  assert.match(x.text(), /Already done in the previous attempt; skipped this time/);
  assert.equal(x.nodes().filter((node) => node.props.accessibilityRole === "text").length, 13);
  x.h.unmount();
});

test("2345 unknown CMA kind stays generic instead of claiming an agent is ready", () => {
  const x = setup("./AssistantCmaOperation.tsx", "AssistantCmaOperation", { operation: { kind: "future_kind", status: "ready", reason: "completed" } });
  assert.match(x.text(), /Activity details unavailable in this app version/);
  assert.doesNotMatch(x.text(), /Agent ready|Result ready/);
  x.h.unmount();
});
