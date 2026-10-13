import { describe, expect, it } from "vitest";
import { assistantFeedbackAdminSchema } from "./assistant-feedback-admin";

const row = {
  message_id: "m",
  vote: "down",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};

describe("assistantFeedbackAdminSchema", () => {
  it("keeps the reply context snapshot", () => {
    const parsed = assistantFeedbackAdminSchema.parse({
      counts: [],
      recent: [
        {
          ...row,
          context: {
            surface: "conversation",
            agent_kind: "specialist",
            model: "nyxagent/chat",
            error_code: "idle_timeout",
            tool_labels: ["github__list_issues"],
            latency_ms: 1500,
            tool_calls: 2,
            tool_errors: 1,
            reply_created_at: "2026-01-01T00:00:00Z",
          },
        },
      ],
    });
    expect(parsed.recent[0]?.context?.tool_labels).toEqual(["github__list_issues"]);
    expect(parsed.recent[0]?.context?.latency_ms).toBe(1500);
  });

  it("accepts pre-A1 rows with no or null context", () => {
    const parsed = assistantFeedbackAdminSchema.parse({
      counts: [],
      recent: [row, { ...row, message_id: "n", context: null }],
    });
    expect(parsed.recent.map((r) => r.context ?? null)).toEqual([null, null]);
  });
});
