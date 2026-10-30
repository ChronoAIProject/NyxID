import assert from "node:assert/strict";
import { test } from "node:test";
import {
  mapBackendRequestToChallenge,
  type BackendApprovalRequestItem,
} from "./approvalAdapter";
import { getChallengeActionState } from "../../features/activity/challengeUiState";

const fixture: BackendApprovalRequestItem = {
  id: "req",
  service_name: "GitHub",
  service_slug: "api-github",
  requester_type: "agent_key",
  requester_label: "Research agent",
  operation_summary: "POST /repos/read",
  http_method: "POST",
  resource: "repository:NyxID",
  verb: "read",
  approval_mode: "per_request",
  status: "pending",
  created_at: "2026-10-13T00:00:00Z",
  expires_at: "2026-10-13T00:02:00Z",
};
test("actual two-minute expiration is preserved and disables a stale pending decision", () => {
  const detail = mapBackendRequestToChallenge(fixture);
  assert.equal(detail.expires_at, fixture.expires_at);
  assert.equal(
    getChallengeActionState({
      ...detail,
      expires_at: new Date(Date.now() - 1).toISOString(),
    }).canDecide,
    false,
  );
});
test("tool metadata has priority over HTTP method and read POST is not marked write", () => {
  const detail = mapBackendRequestToChallenge({
    ...fixture,
    tool_name: "search_repositories",
    tool_arguments: '{"query":"nyxid"}',
  });
  assert.equal(detail.action, "search_repositories");
  assert.equal(detail.risk_level, "low");
  assert.equal(detail.resource, "repository:NyxID");
  assert.equal(detail.tool_arguments, '{"query":"nyxid"}');
  assert.equal(detail.request_context.requester, "Research agent");
  assert.equal("location" in detail.request_context, false);
});
test("explicit destructive tool is high risk even when carried by GET", () => {
  const detail = mapBackendRequestToChallenge({
    ...fixture,
    http_method: "GET",
    is_destructive: true,
  });
  assert.equal(detail.risk_level, "high");
});
test("missing expiry fails closed and missing semantic metadata stays unknown", () => {
  const detail = mapBackendRequestToChallenge({
    ...fixture,
    expires_at: undefined,
    verb: undefined,
  });
  assert.equal(detail.expires_at, "");
  assert.equal(detail.risk_level, "unknown");
  assert.equal(getChallengeActionState(detail).canDecide, false);
});
test("only authoritative org policy flag exposes org context", () => {
  assert.equal(
    mapBackendRequestToChallenge({ ...fixture, org_id: "org" }).org_id,
    undefined,
  );
  assert.equal(
    mapBackendRequestToChallenge({
      ...fixture,
      from_org_policy: true,
      org_id: "org",
    }).org_id,
    "org",
  );
});
test("history preserves the decision timestamp independently of request creation", () => {
  const decidedAt = "2026-10-13T00:01:00Z";
  const detail = mapBackendRequestToChallenge({
    ...fixture,
    status: "approved",
    decided_at: decidedAt,
    decision_channel: "web",
  });
  assert.equal(detail.status, "APPROVED");
  assert.equal(detail.created_at, fixture.created_at);
  assert.equal(detail.decided_at, decidedAt);
  assert.equal(detail.decision_channel, "web");
});
test("decision channels match Web semantics including push as Mobile", () => {
  for (const [channel, expected] of [
    ["web", "web"],
    ["telegram", "telegram"],
    ["mobile", "mobile"],
    ["push", "mobile"],
  ] as const) {
    assert.equal(
      mapBackendRequestToChallenge({
        ...fixture,
        status: "rejected",
        decision_channel: channel,
      }).decision_channel,
      expected,
    );
  }
});
test("legacy or unknown decision metadata remains unknown instead of inventing a decision source", () => {
  const legacy = mapBackendRequestToChallenge(fixture);
  assert.equal(legacy.decided_at, null);
  assert.equal(legacy.decision_channel, null);
  const unknown = mapBackendRequestToChallenge({
    ...fixture,
    status: "expired",
    decided_at: null,
    decision_channel: "future-channel",
  });
  assert.equal(unknown.decided_at, null);
  assert.equal(unknown.decision_channel, null);
});
test("requester falls back to the real requester type when an agent label is absent", () => {
  assert.equal(
    mapBackendRequestToChallenge({ ...fixture, requester_label: null })
      .request_context.requester,
    fixture.requester_type,
  );
});
