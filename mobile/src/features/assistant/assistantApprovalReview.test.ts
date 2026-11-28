import assert from "node:assert/strict";
import test from "node:test";
import { assistantApprovalReviewPath } from "./assistantApprovalReview";
import { historySchema } from "./assistantTypes";

const id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const conversation = `nyxa-${"a".repeat(32)}`;
const group = `nyxg-${"b".repeat(32)}`;
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
  for (const [version, reviewPath] of [[1, path], [3, path], [4, path], [5, path], [2, "/assistant/approvals"], [1, `/assistant?c=${group}`], [3, `/assistant?c=${group}`], [4, `/assistant?c=${group}`], [5, `/assistant?c=${group.toUpperCase().replace("NYXG", "nyxg")}`]] as const) {
    const decoded = row({ ...proposal, version, review_path: reviewPath }, reviewPath);
    assert.equal(decoded.execution_proposal?.version, version);
    assert.equal(decoded.review_path, reviewPath);
    assert.equal(assistantApprovalReviewPath(decoded), reviewPath);
  }
});

test("every proposal writer state survives history decoding for every supported version", () => {
  // v1/v3/v4/v5 share resume/lifecycle writers; v2 has its own delivery/watch writers.
  const states = ["pending", "queued", "preparing", "dispatched", "waiting", "completed", "denied", "expired", "cancelled", "not_sent", "unknown"];
  for (const version of [1, 2, 3, 4, 5]) for (const state of states) {
    const reviewPath = version === 2 ? "/assistant/approvals" : path;
    const decoded = row({ ...proposal, version, state, can_decide: false, review_path: reviewPath });
    assert.equal(decoded.execution_proposal?.state, state, `v${version} ${state}`);
    assert.equal(decoded.execution_proposal?.can_decide, false);
    assert.equal(assistantApprovalReviewPath(decoded), reviewPath);
  }
});

test("group review bounds match real nyxg IDs and never accept guessed UUID routes", () => {
  for (const target of [id, "b".repeat(32), "nyxg-" + "b".repeat(31), "nyxg-" + "z".repeat(32), "NYXG-" + "b".repeat(32), `${group}&card=${id}`, `${group}&c=${group}`, `${group}&g=${group}`]) {
    assert.equal(assistantApprovalReviewPath(row({ ...proposal, review_path: `/assistant?c=${target}` })), undefined, target);
  }
});

test("bad proposal metadata never becomes an ordinary approval or gains a Review link", () => {
  for (const value of [false, 0, "future", [], {}, { ...proposal, version: 999 }, { ...proposal, supported: false }, { ...proposal, supported: "true" }, { ...proposal, can_decide: undefined }, { ...proposal, state: "unsupported" }, { ...proposal, state: "failed" }, { ...proposal, state: "future_state" }]) {
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
