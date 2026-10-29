import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "./assistantHookTestHarness";
import {
  acknowledgementSchema,
  type AssistantAcknowledgement,
} from "./assistantTypes";

type Element = { type: unknown; props: Record<string, any>; key?: unknown };
const A = "d9428888-122b-41e1-85c5-61cd3cbb3210";
const B = "d9428888-122b-41e1-85c5-61cd3cbb3211";

function card(
  id: string,
  status: AssistantAcknowledgement["status"] = "pending",
) {
  return acknowledgementSchema.parse({
    id,
    status,
    kind: "action",
    summary: id === A ? "Create Twitter assistant" : "Add Twitter skill",
    service_name: "Twitter / X API",
    service_slug: "api-twitter",
    tool_name: "nyxid__spawn_subagent",
    decided_by: status === "pending" ? null : "user",
    confirmation: {
      risk: "sensitive",
      reason: "access_mode",
      access_mode: "nyxmode",
    },
    created_at: "2026-10-16T00:00:00Z",
    expires_at: "2026-10-16T00:15:00Z",
  });
}

function elements(element: unknown): Element[] {
  if (!element || typeof element !== "object" || !("props" in element))
    return [];
  const node = element as Element;
  return [node, ...[node.props.children].flat(Infinity).flatMap(elements)];
}
function text(element: unknown): string {
  if (typeof element === "string" || typeof element === "number")
    return String(element);
  if (!element || typeof element !== "object" || !("props" in element))
    return "";
  return [(element as Element).props.children]
    .flat(Infinity)
    .map(text)
    .join(" ");
}

function setup(
  initialRows: AssistantAcknowledgement[],
  initialSelected?: string,
) {
  let rows = initialRows;
  let selectedId = initialSelected;
  let toggles = 0;
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
      jsxs: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      Pressable: "Pressable",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": {
      ChevronDown: "ChevronDown",
      ChevronRight: "ChevronRight",
      History: "History",
    },
    "../../components/workspace/WorkspaceUI": { Label: "Label" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./AssistantReviewTarget": { AssistantReviewTarget: "ReviewTarget" },
  });
  const Component = harness.load(
    fileURLToPath(new URL("./AssistantAcknowledgements.tsx", import.meta.url)),
  ).AssistantAcknowledgements as (props: any) => Element;
  harness.mount(() =>
    Component({
      acknowledgements: rows,
      selectedId,
      renderPending: (row: AssistantAcknowledgement) => ({
        type: "PendingCard",
        key: row.id,
        props: { row },
      }),
      renderAuthoredReview: (row: AssistantAcknowledgement) => ({
        type: "AuthoredReview",
        props: { row, initiallyOpen: row.id === selectedId },
      }),
      onToggle: () => {
        toggles++;
      },
    }),
  );
  const all = () => elements(harness.value());
  const press = (label: string) => {
    const target = all().find(
      (element) => element.props.accessibilityLabel === label,
    );
    assert.ok(target, `Missing ${label}`);
    target.props.onPress();
    harness.flush();
  };
  return {
    harness,
    all,
    press,
    text: () => text(harness.value()),
    toggles: () => toggles,
    update: (next: AssistantAcknowledgement[], nextSelected = selectedId) => {
      rows = next;
      selectedId = nextSelected;
      harness.render();
    },
  };
}

test("only server-pending requests receive full actionable cards; settled confirmations take one collapsed row", () => {
  const x = setup([card(A), card(B, "used")]);
  assert.equal(x.all().filter((node) => node.type === "PendingCard").length, 1);
  assert.ok(
    x
      .all()
      .find(
        (node) => node.props.accessibilityLabel === "1 confirmation record",
      ),
  );
  assert.doesNotMatch(
    x.text(),
    /Create Twitter|Add Twitter|Tool:|NyxMode|Confirm this action/,
  );
  assert.equal(x.toggles(), 0);
  x.harness.unmount();
});

test("authoritative pending-to-used refresh removes the full card even when it was selected; an unchanged pending snapshot stays actionable", () => {
  const x = setup([card(A)], A);
  x.update([card(A)]);
  assert.equal(x.all().filter((node) => node.type === "PendingCard").length, 1);
  x.update([card(A, "used")]);
  assert.equal(x.all().filter((node) => node.type === "PendingCard").length, 0);
  assert.doesNotMatch(x.text(), /Create Twitter|This one-time/);
  assert.equal(
    x
      .all()
      .find((node) => node.props.accessibilityLabel === "1 confirmation record")
      ?.props.accessibilityState.expanded,
    false,
  );
  x.harness.unmount();
});

test("history exposes compact decisions and optional audit details without repeating confirmation prompts", () => {
  const x = setup([card(A, "used"), card(B, "denied")]);
  x.press("2 confirmation records");
  assert.match(x.text(), /Used.*Create Twitter assistant/);
  assert.match(x.text(), /Denied by you.*Add Twitter skill/);
  assert.doesNotMatch(
    x.text(),
    /Tool:|Confirm this action|requires this confirmation/,
  );
  x.press("Used · Create Twitter assistant");
  assert.match(x.text(), /one-time confirmation has been used/);
  assert.match(
    x.text(),
    /Check the assistant’s response for the action’s result/,
  );
  assert.match(x.text(), /Tool:.*nyxid__spawn_subagent/);
  assert.match(x.text(), /Risk:.*sensitive/);
  assert.match(x.text(), /Decided by.*you/);
  assert.match(x.text(), /Confirmation:.*d9428888/);
  assert.doesNotMatch(
    x.text(),
    /Confirm this action|requires this confirmation/,
  );
  x.press("2 confirmation records");
  assert.doesNotMatch(x.text(), /Create Twitter assistant|Tool:/);
  assert.equal(x.toggles(), 3);
  x.harness.unmount();
});

test("a completed-card deep link arriving before history opens its exact audit record and preserves authored review recovery", () => {
  const x = setup([], B);
  const authored = {
    ...card(B, "expired"),
    authored_skill: { agent_id: "agent-1", proposal_id: "proposal-1" },
  };
  x.update([card(A, "used"), authored]);
  assert.ok(
    x.all().some((node) => node.type === "ReviewTarget" && node.props.selected),
  );
  const review = x.all().find((node) => node.type === "AuthoredReview");
  assert.equal(review?.props.row.id, B);
  assert.equal(review?.props.initiallyOpen, true);
  x.update([card(A, "used"), authored], A);
  assert.equal(
    x.all().find((node) => node.type === "AuthoredReview"),
    undefined,
  );
  assert.match(x.text(), /one-time confirmation has been used/);
  x.harness.unmount();
});

test("an expired authored review remains reachable through history without an automatic decision", () => {
  const x = setup([
    {
      ...card(A, "expired"),
      authored_skill: { agent_id: "agent-1", proposal_id: "proposal-1" },
    },
  ]);
  assert.equal(
    x.all().find((node) => node.type === "AuthoredReview"),
    undefined,
  );
  x.press("1 confirmation record");
  x.press("Expired · Create Twitter assistant");
  assert.equal(
    x.all().find((node) => node.type === "AuthoredReview")?.props.row.id,
    A,
  );
  x.harness.unmount();
});
