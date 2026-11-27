import { describe, expect, it } from "vitest";
import type { NyxAgentAcknowledgement, NyxAgentApproval } from "@/schemas/assistant-nyxagent";
import { authoredSkillPreviewSchema } from "@/schemas/agent-skills";
import { nextActiveDecision, pendingChatDecisions } from "./pending-decisions";

const now = Date.parse("2026-11-26T00:00:00Z");
const ack: NyxAgentAcknowledgement = {
  id: "ack", kind: "action", status: "pending", summary: "Create a document", decider: "user", decided_by: null,
  reason: null, service_slug: null, service_name: null, tool_name: "create", created_at: "2026-11-25T23:59:00Z",
  decided_at: null, expires_at: "2026-11-26T00:01:00Z",
};
const approval: NyxAgentApproval = {
  id: "proxy", status: "pending", service_slug: "sheets", service_name: "Sheets", summary: "POST /spreadsheets",
  approval_mode: "per_request", agent_key_prefix: "", created_at: "2026-11-25T23:58:00Z", expires_at: ack.expires_at,
};
function select(overrides: Partial<Parameters<typeof pendingChatDecisions>[0]> = {}) {
  return pendingChatDecisions({ acknowledgements: [ack], approvals: [], consents: [], now, ...overrides });
}

describe("human decisions", () => {
  it("counts only live user decisions, ordered by their original request", () => {
    expect(select({ approvals: [approval] }).map((row) => row.id)).toEqual(["approval:proxy", "acknowledgement:ack"]);
    expect(select({ acknowledgements: [{ ...ack, decider: "orchestrator" }] })).toEqual([]);
    expect(select({ now: Date.parse(ack.expires_at) })).toEqual([]);
  });
  it.each(["allowed", "used", "denied", "expired"] as const)("leaves %s acknowledgements in history", (status) => {
    expect(select({ acknowledgements: [{ ...ack, status }] })).toEqual([]);
  });
  it.each(["access_mode", "webhook", "policy", "approval", "org_connection", "delegation"] as const)("preserves the Full access floor for %s", (reason) => {
    const rows = select({ accessMode: "autopilot", acknowledgements: [{ ...ack, confirmation: { risk: "change", reason } }] });
    expect(rows.length).toBe(["org_connection", "delegation"].includes(reason) ? 1 : 0);
  });
  it.each(["approved", "rejected", "expired"] as const)("keeps %s proxy approvals out of the stack", (status) => {
    expect(select({ acknowledgements: [], approvals: [{ ...approval, status }] })).toEqual([]);
  });
  it("keeps legacy proxy requests with missing status or unknown expiry reviewable", () => {
    expect(select({ acknowledgements: [], approvals: [{ ...approval, status: undefined, expires_at: "" }] })).toHaveLength(1);
    expect(select({ acknowledgements: [], approvals: [{ ...approval, expires_at: "2026-11-25T23:59:00Z" }] })).toEqual([]);
  });
  it("does not count saved, unsupported, or non-decidable execution permissions", () => {
    const proposal = { version: 1, supported: true, revision: 1, state: "pending", can_decide: true, review_path: "", review: null, choices: [], grant_id: null, failure_code: null };
    for (const update of [{ state: "queued" }, { supported: false }, { can_decide: false }]) {
      expect(select({ acknowledgements: [], approvals: [{ ...approval, execution_proposal: { ...proposal, ...update } }] })).toEqual([]);
    }
    expect(select({ acknowledgements: [], approvals: [{ ...approval, execution_proposal: proposal }] })).toHaveLength(1);
  });
  it("keeps authored publication recovery in history and a fresh review in the stack", () => {
    const card = { ...ack, authored_skill: { agent_id: "agent", proposal_id: "proposal", revision: 0, skills_revision: 0 } };
    const preview = authoredSkillPreviewSchema.parse({ id: "proposal", agent_id: "agent", agent_name: "Bot", revision: 0, skills_revision: 0, current_skills_revision: 0, status: "pending", name: "test", version: "1.0", files: [], actions: ["deny", "publish"] });
    expect(select({ acknowledgements: [card] })).toHaveLength(1);
    for (const data of [{ ...preview, state: "pinned" }, { ...preview, actions: ["retry", "check"] }, { ...preview, state: "invalidated" }]) {
      expect(select({ acknowledgements: [card], authoredPreviews: new Map([[card.id, { data }]]) })).toEqual([]);
    }
    expect(select({ acknowledgements: [card], authoredPreviews: new Map([[card.id, { data: preview }]]) })).toHaveLength(1);
  });
  it("includes only pending, unexpired shared connection decisions", () => {
    const consent = { id: "consent", connection_id: "connection", catalog_service_id: "service", operation: "POST /events", conversation_id: "chat", decision: "pending" as const, expires_at: null, scope: "operation" };
    expect(select({ acknowledgements: [], consents: [consent] }).map((row) => row.id)).toEqual(["consent:consent"]);
    expect(select({ acknowledgements: [], consents: [{ ...consent, decision: "once" }, { ...consent, id: "expired", expires_at: "2026-11-25T00:00:00Z" }] })).toEqual([]);
  });
  it("shows the newest authored review after an edit or renewal, even if the old preview is cached as active", () => {
    const old = { ...ack, authored_skill: { agent_id: "agent", proposal_id: "proposal", revision: 0, skills_revision: 0 } };
    const edited = { ...old, id: "edited", authored_skill: { ...old.authored_skill, revision: 1 } };
    expect(select({ acknowledgements: [edited, old] }).map((row) => row.id)).toEqual(["acknowledgement:edited"]);
    const renewed = { ...edited, id: "renewed", created_at: "2026-11-25T23:59:30Z" };
    expect(select({ acknowledgements: [old, renewed, edited] }).map((row) => row.id)).toEqual(["acknowledgement:renewed"]);
    expect(select({ acknowledgements: [old, { ...edited, status: "denied" }] })).toEqual([]);
  });
});

it("keeps the active identity on arrivals and advances to a surviving successor or predecessor on removal", () => {
  expect(nextActiveDecision("b", ["a", "b", "c"], ["new", "a", "b", "c"])).toBe("b");
  expect(nextActiveDecision("b", ["a", "b", "c"], ["new", "a", "c"])).toBe("c");
  expect(nextActiveDecision("c", ["a", "b", "c"], ["new", "a", "b"])).toBe("b");
  expect(nextActiveDecision("a", ["a"], [])).toBeUndefined();
});
