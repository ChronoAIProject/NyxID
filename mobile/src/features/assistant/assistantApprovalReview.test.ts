import assert from "node:assert/strict";
import test from "node:test";
import { assistantApprovalReviewPath } from "./assistantApprovalReview";
import { historySchema } from "./assistantTypes";

const id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const conversation = `nyxa-${"a".repeat(32)}`;
const path = `/assistant?c=${conversation}&card=${id}`;
const proposal = { version: 4, state: "pending", supported: true, can_decide: true, review_path: path };
const raw = {
  conversation: { id: conversation, title: "Review", model: "nyxagent/chat", created_at: "2026-10-28", last_message_at: "2026-10-28", message_count: 0, active_turn: null },
  messages: [], before_seq: null,
};
function row(value: unknown, topPath?: string) {
  return historySchema.parse({ ...raw, approvals: [{ id, service_name: "Sheets", summary: "Write cells", expires_at: "2099-01-01", execution_proposal: value, review_path: topPath }] }).approvals[0]!;
}

test("assistant history retains every server-produced proposal review destination exactly", () => {
  for (const [version, reviewPath] of [[1, path], [3, path], [4, path], [5, path], [2, "/assistant/approvals"], [1, `/assistant?c=${id}`], [3, `/assistant?c=${id}`]] as const) {
    const decoded = row({ ...proposal, version, review_path: reviewPath }, reviewPath);
    assert.equal(decoded.execution_proposal?.version, version);
    assert.equal(decoded.review_path, reviewPath);
    assert.equal(assistantApprovalReviewPath(decoded), reviewPath);
  }
});

test("bad proposal metadata never becomes an ordinary approval or gains a Review link", () => {
  for (const value of [false, 0, "future", [], {}, { ...proposal, version: 999 }, { ...proposal, supported: false }, { ...proposal, supported: "true" }, { ...proposal, can_decide: undefined }, { ...proposal, state: "unsupported" }, { ...proposal, state: "future_state" }]) {
    const decoded = row(value);
    assert.ok(decoded.execution_proposal);
    assert.equal(assistantApprovalReviewPath(decoded), undefined);
  }
  for (const reviewPath of ["https://evil.test" + path, "//evil.test" + path, path + "#fragment", path + "&card=" + id, path.replace(id, "foreign-card"), path + "&next=https://evil.test", "/assistant/approvals?request=" + id, "/assistant?c=" + conversation, path + "\\outside", "/assistant/../assistant?c=" + conversation + "&card=" + id]) {
    assert.equal(assistantApprovalReviewPath(row({ ...proposal, review_path: reviewPath })), undefined, reviewPath);
  }
  assert.equal(assistantApprovalReviewPath(row(proposal, "/assistant?c=" + id)), undefined);
  assert.equal(assistantApprovalReviewPath(row({ ...proposal, version: 2 })), undefined);
});

test("ordinary rows retain no proposal; terminal proposals retain their state without native decision authority", () => {
  assert.equal(row(undefined).execution_proposal, undefined);
  assert.equal(row(null).execution_proposal, null);
  for (const state of ["denied", "expired", "completed", "unknown"]) {
    const decoded = row({ ...proposal, state, can_decide: false });
    assert.equal(decoded.execution_proposal?.state, state);
    assert.equal(decoded.execution_proposal?.can_decide, false);
  }
});
