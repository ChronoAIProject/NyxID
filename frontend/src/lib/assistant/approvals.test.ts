import { describe, expect, it } from "vitest";
import { toAssistantApprovalEntry } from "./approvals";
import type { ApprovalRequestItem } from "@/types/approvals";

function request(
  overrides: Partial<ApprovalRequestItem> = {},
): ApprovalRequestItem {
  return {
    id: "req-1",
    service_name: "Lark Bot",
    service_slug: "lark-bot",
    requester_type: "api_key",
    requester_label: "coding-agent",
    operation_summary: "POST /messages",
    action_description: "Post the drafted summary to #payments-oncall",
    approval_mode: "per_request",
    status: "pending",
    created_at: "2026-07-16T00:00:00+00:00",
    expires_at: "2026-07-16T00:15:00+00:00",
    decided_at: null,
    decision_channel: null,
    ...overrides,
  };
}

describe("toAssistantApprovalEntry", () => {
  it("maps a pending request to an undecided approval card", () => {
    const entry = toAssistantApprovalEntry(request());
    expect(entry).toMatchObject({
      requestId: "req-1",
      serviceName: "Lark Bot",
      requestedAt: "2026-07-16T00:00:00+00:00",
      decidedAt: null,
    });
    expect(entry.block).toMatchObject({
      type: "approval_card",
      block_id: "req-1",
      approval_request_id: "req-1",
      body: "Post the drafted summary to #payments-oncall",
      service_slug: "lark-bot",
      agent_key_prefix: "coding-agent",
      approval_mode: "per_request",
      grant_duration_sec: null,
      expires_at: "2026-07-16T00:15:00+00:00",
      decision: null,
      decision_channel: null,
    });
  });

  it("translates NyxID 'rejected' to the block spelling 'denied'", () => {
    const entry = toAssistantApprovalEntry(
      request({
        status: "rejected",
        decided_at: "2026-07-16T00:05:00+00:00",
        decision_channel: "telegram",
      }),
    );
    expect(entry.decidedAt).toBe("2026-07-16T00:05:00+00:00");
    expect(entry.block.decision).toBe("denied");
    expect(entry.block.decision_channel).toBe("telegram");
  });

  it("maps the 'push' decide channel to the mobile channel", () => {
    const entry = toAssistantApprovalEntry(
      request({ status: "approved", decision_channel: "push" }),
    );
    expect(entry.block.decision).toBe("approved");
    expect(entry.block.decision_channel).toBe("mobile");
  });

  it("drops unknown decision channels instead of guessing", () => {
    const entry = toAssistantApprovalEntry(
      request({ status: "expired", decision_channel: "carrier-pigeon" }),
    );
    expect(entry.block.decision).toBe("expired");
    expect(entry.block.decision_channel).toBeNull();
  });

  it("only carries the grant duration for grant-mode requests", () => {
    const grant = toAssistantApprovalEntry(
      request({ approval_mode: "grant" }),
      7 * 86_400,
    );
    expect(grant.block.grant_duration_sec).toBe(7 * 86_400);
    const perRequest = toAssistantApprovalEntry(request(), 7 * 86_400);
    expect(perRequest.block.grant_duration_sec).toBeNull();
  });

  it("falls back to requester_type and operation_summary when labels are missing", () => {
    const entry = toAssistantApprovalEntry(
      request({ requester_label: null, action_description: null }),
    );
    expect(entry.block.agent_key_prefix).toBe("api_key");
    expect(entry.block.body).toBe("POST /messages");
  });

  it("describes tool approvals by their arguments", () => {
    const entry = toAssistantApprovalEntry(
      request({
        tool_name: "delete_repo",
        tool_arguments: '{"repo":"nyxid"}',
        action_description: null,
      }),
    );
    expect(entry.block.body).toBe('{"repo":"nyxid"}');
  });
});
