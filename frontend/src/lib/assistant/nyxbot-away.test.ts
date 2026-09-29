import { describe, expect, it } from "vitest";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { awayItems } from "./nyxbot-away";

const at = (minute: number) => `2026-09-29T10:${String(minute).padStart(2, "0")}:00Z`;

function agent(fields: Partial<AssistantAgent>): AssistantAgent {
  return {
    id: "agent",
    kind: "specialist",
    name: "agent",
    description: "",
    display_name: null,
    persona: null,
    specialty: null,
    created_by: "user",
    status: "idle",
    services: [],
    account_read: false,
    pending_requests: [],
    last_reply: null,
    home_conversation_id: null,
    memory_count: 0,
    created_at: at(0),
    last_active_at: at(0),
    destroyed_at: null,
    pending_acknowledgements: 0,
    channels: [],
    ...fields,
  };
}

describe("awayItems", () => {
  it("puts requests first, then replies and current work newest first", () => {
    const items = awayItems([
      agent({ id: "nyxbot", kind: "nyxbot", name: "NyxBot", last_reply: { seq: 1, status: "completed", text: "hi", created_at: at(50) } }),
      agent({ id: "old", name: "old", last_reply: { seq: 1, status: "completed", text: "old reply", created_at: at(5) } }),
      agent({ id: "busy", name: "busy", status: "running", last_active_at: at(30) }),
      agent({
        id: "asker",
        name: "asker",
        last_reply: { seq: 2, status: "completed", text: "new reply", created_at: at(20) },
        pending_requests: [
          {
            request_id: "r1",
            agent: "asker",
            agent_id: "asker",
            conversation_id: `nyxa-${"a".repeat(32)}`,
            kind: "service",
            service_slug: "slack",
            summary: "Use Slack",
            requested_by: null,
            expires_at: at(59),
          },
        ],
      }),
      agent({ id: "gone", name: "gone", status: "destroyed", last_reply: { seq: 1, status: "completed", text: "x", created_at: at(59) } }),
      agent({ id: "quiet", name: "quiet" }),
    ]);
    expect(items.map((item) => item.key)).toEqual([
      "request:r1",
      "update:busy",
      "update:asker",
      "update:old",
    ]);
    const busy = items[1];
    expect(busy?.kind === "update" && busy.text).toBe("Working on a task.");
  });
});
