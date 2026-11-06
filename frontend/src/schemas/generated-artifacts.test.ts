import { expect, it } from "vitest";
import {
  nyxAgentAttachmentSchema,
  nyxAgentMessageSchema,
} from "./assistant-nyxagent";
it("accepts future attachment and delivery metadata without rejecting the reply", () => {
  const attachment = {
    id: "future",
    label: "file",
    size: 12,
    content_type: "application/future",
    origin: "future_origin",
    preview_kind: "future_preview",
  };
  expect(nyxAgentAttachmentSchema.parse(attachment).preview_kind).toBe(
    "future_preview",
  );
  const message = {
    id: "reply",
    seq: 1,
    turn_id: "turn",
    role: "assistant",
    error_code: null,
    text: "hello",
    status: "completed",
    created_at: "2026-11-05T00:00:00Z",
    attachments: [attachment],
    artifact_delivery: {
      state: "future",
      count: 100,
      recovery_available: false,
      rejected: Array.from({ length: 12 }, (_, index) => ({
        index: index + 100,
        filename: "file",
        reason: "future_reason",
      })),
    },
  };
  expect(
    nyxAgentMessageSchema.parse(message).artifact_delivery?.rejected,
  ).toHaveLength(12);
});
