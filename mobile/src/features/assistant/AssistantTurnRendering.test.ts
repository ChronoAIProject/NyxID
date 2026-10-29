import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "./assistantHookTestHarness";
import { activitySchema, messageSchema } from "./assistantTypes";

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
      X: "X",
      ChevronDown: "ChevronDown",
      ChevronRight: "ChevronRight",
    },
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
  x.press("Details");
  assert.match(x.text(), /stage: stream/);
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
