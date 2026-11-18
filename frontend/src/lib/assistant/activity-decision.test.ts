import { describe, expect, it } from "vitest";
import { countDecisions, decisionSummary } from "./turn-decisions";
import { activityDecisionPresentation } from "./activity-decision";
import { nyxAgentAcknowledgementSchema, nyxAgentTurnActivitySchema } from "@/schemas/assistant-nyxagent";

describe("activity decision compatibility and count identity", () => {
  it("does not reinterpret legacy errors", () => {
    const row = nyxAgentTurnActivitySchema.parse({ id: "a", label: "tool", status: "error", started_at: "now" });
    expect(activityDecisionPresentation(row.decision)).toBeUndefined();
  });
  it("counts the linked receipt once across the activity and card", () => {
    const card = nyxAgentAcknowledgementSchema.parse({ id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", service_slug: null, service_name: null, tool_name: null, decided_at: null, kind: "action", status: "used", summary: "Confirm", created_at: "now", expires_at: "later" });
    const link = { id: card.id, kind: "acknowledgement", state: "completed" };
    expect(decisionSummary(countDecisions([card], [link, link]))).toEqual(["1 allowed"]);
  });
  it("future states degrade to unconfirmed instead of success or failure", () => {
    expect(activityDecisionPresentation({ id: "card", kind: "future", state: "future" })?.status).toBe("waiting");
  });
});
