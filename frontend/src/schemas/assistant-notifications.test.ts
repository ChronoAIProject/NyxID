import { describe, expect, it } from "vitest";
import {
  NOTIFICATION_FILTER_KINDS,
  notificationFilter,
  notificationListSchema,
  notificationSettingsSchema,
  notificationSummarySchema,
} from "./assistant-notifications";

const row = {
  id: "n-1",
  kind: "agent_update",
  count: 3,
  source: { type: "conversation", id: "nyxa-1", label: "Research" },
  title: "Research replied",
  preview: "Done: three candidates found.",
  url: "https://nyxid.example/assistant?c=nyxa-1",
  read: false,
  resolved: false,
  created_at: "2026-11-10T08:00:00Z",
  updated_at: "2026-11-10T08:05:00Z",
};

describe("notification list", () => {
  it("defaults optional source identifiers and keeps the mark-all fence", () => {
    const page = notificationListSchema.parse({ items: [row], next_cursor: null, as_of: "2026-11-10T08:06:00Z" });
    expect(page.items[0]?.source).toEqual({
      type: "conversation", id: "nyxa-1", label: "Research",
      conversation_id: null, group_id: null, channel_id: null, trigger_id: null, agent_id: null,
    });
    expect(page.as_of).toBe("2026-11-10T08:06:00Z");
    expect(page.next_cursor).toBeNull();
  });

  it("drops a row of an unknown kind or source type without losing the page", () => {
    const page = notificationListSchema.parse({
      items: [row, { ...row, id: "n-2", kind: "future_kind" }, { ...row, id: "n-3", source: { type: "mystery", id: "x" } }],
      next_cursor: "cursor-2",
      as_of: "2026-11-10T08:06:00Z",
    });
    expect(page.items.map((item) => item.id)).toEqual(["n-1"]);
    expect(page.next_cursor).toBe("cursor-2");
  });

  it("requires the fence the mark-all action depends on", () => {
    expect(notificationListSchema.safeParse({ items: [], next_cursor: null }).success).toBe(false);
  });
});

describe("notification summary", () => {
  it("keeps additive conversation links on channel sources and drops unknown source types", () => {
    const summary = notificationSummarySchema.parse({
      unread_count: 2,
      needs_decision_count: 1,
      by_source: [
        { source_type: "channel_chat", id: "thread-1", unread: 1, conversation_id: "nyxa-9" },
        { source_type: "mystery", id: "x", unread: 1 },
      ],
      by_source_truncated: true,
    });
    expect(summary.by_source).toEqual([
      { source_type: "channel_chat", id: "thread-1", unread: 1, conversation_id: "nyxa-9" },
    ]);
    expect(summary.by_source_truncated).toBe(true);
  });

  it("reads an empty summary from an older server", () => {
    expect(notificationSummarySchema.parse({})).toEqual({ unread_count: 0, needs_decision_count: 0, by_source: [] });
  });
});

describe("filters and settings", () => {
  it("maps every chip to a closed kind subset, with All unfiltered", () => {
    expect(NOTIFICATION_FILTER_KINDS.all).toEqual([]);
    expect(NOTIFICATION_FILTER_KINDS.needs_you).toEqual(["needs_decision"]);
    expect(NOTIFICATION_FILTER_KINDS.automations).toEqual(["automation_result", "automation_problem"]);
    expect(NOTIFICATION_FILTER_KINDS.chats).toEqual(["chat_update", "agent_update", "group_mention"]);
    expect(notificationFilter("chats")).toBe("chats");
    expect(notificationFilter("everything")).toBeUndefined();
  });

  it("defaults every push switch off", () => {
    expect(notificationSettingsSchema.parse({}).push).toEqual({
      automation_result: false, automation_problem: false, needs_decision: false,
      agent_update: false, chat_update: false, group_mention: false,
    });
    expect(notificationSettingsSchema.parse({ push: { chat_update: true } }).push.chat_update).toBe(true);
  });
});
