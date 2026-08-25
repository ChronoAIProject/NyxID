import { describe, expect, it } from "vitest";
import {
  ACTION_REQUEST_CONFLICT_NOTE,
  ACTION_REQUEST_UNREPORTED_COMPLETED_NOTE,
} from "@/lib/assistant/action-notes";
import { actionSummaryBlock } from "@/lib/assistant/chat-action-presentation";
import type { ChatActionSummary } from "@/lib/assistant/chat-actor-state";

const BASE_SUMMARY: ChatActionSummary = {
  schemaVersion: 4,
  actorId: "nyxid-chat-action-copy",
  originTurnId: "turn-action-copy",
  taskId: "task-action-copy",
  stepId: "step-action-copy",
  actionRequestId: "request-action-copy",
  action: "service.connect",
  request: null,
  supported: true,
  recovered: false,
};

function summary(
  disposition?: string,
  overrides: Partial<ChatActionSummary> = {},
): ChatActionSummary {
  return {
    ...BASE_SUMMARY,
    ...(disposition ? { reports: [{ disposition }] } : {}),
    ...overrides,
  };
}

describe("action summary presentation", () => {
  it("uses the accepted-delivery copy for a completed action", () => {
    expect(actionSummaryBlock(summary("completed"))).toMatchObject({
      status: "completed",
      outcome_note: "Reported — awaiting assistant verification.",
    });
  });

  it("uses the accepted-delivery copy for a declined action", () => {
    expect(actionSummaryBlock(summary("declined"))).toMatchObject({
      status: "declined",
      outcome_note:
        "You declined this request. The assistant received the decision; no credential was shared.",
    });
  });

  it.each(["failed", "cancelled", "expired"])(
    "uses the accepted-delivery failure copy for a %s action",
    (disposition) => {
      expect(actionSummaryBlock(summary(disposition))).toMatchObject({
        status: "failed",
        outcome_note:
          "The assistant received the connection failure. Ask it to request the service again.",
      });
    },
  );

  it("uses the stable conflict copy for a conflicted request", () => {
    expect(
      actionSummaryBlock(summary(undefined, { conflicted: true })),
    ).toMatchObject({
      status: "conflicted",
      outcome_note: ACTION_REQUEST_CONFLICT_NOTE,
    });
  });

  it("adds the unreported warning when a completed report conflicts", () => {
    expect(
      actionSummaryBlock(summary("completed", { conflicted: true })),
    ).toMatchObject({
      status: "conflicted",
      outcome_note: `${ACTION_REQUEST_CONFLICT_NOTE} ${ACTION_REQUEST_UNREPORTED_COMPLETED_NOTE}`,
    });
  });

  it("keeps a local blocked override note", () => {
    expect(
      actionSummaryBlock(summary(), {
        status: "blocked",
        note: "The connection check timed out.",
      }),
    ).toMatchObject({
      status: "blocked",
      outcome_note: "The connection check timed out.",
    });
  });

  it("keeps a local unsupported override note", () => {
    expect(
      actionSummaryBlock(summary(undefined, { supported: false }), {
        status: "unsupported",
        note: "This action is not supported by this client.",
      }),
    ).toMatchObject({
      status: "unsupported",
      outcome_note: "This action is not supported by this client.",
    });
  });
});
