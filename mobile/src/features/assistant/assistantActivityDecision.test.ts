import assert from "node:assert/strict";
import test from "node:test";
import { activityDecisionPresentation } from "./assistantActivityDecision";
import { countDecisions, decisionSummary } from "./assistantDecisions";
import { acknowledgementSchema } from "./assistantTypes";

test("legacy and unknown decisions are conservative", () => {
  assert.equal(activityDecisionPresentation(undefined), undefined);
  assert.equal(activityDecisionPresentation({ id: "x", kind: "future", state: "future" })?.status, "waiting");
});
test("activity and card count one approval", () => {
  const card = acknowledgementSchema.parse({ id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", service_slug: null, service_name: null, tool_name: null, decided_at: null, kind: "action", status: "used", summary: "Confirm", created_at: "now", expires_at: "later" });
  const link = { id: card.id, kind: "acknowledgement", state: "completed" };
  assert.deepEqual(decisionSummary(countDecisions([card], [link, link])), ["1 allowed"]);
});
