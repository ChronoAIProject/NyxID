import assert from "node:assert/strict";
import test from "node:test";
import {
  notificationAccessibilityLabel,
  notificationBadge,
  notificationBellLabel,
  notificationCountLabel,
  notificationIdFromPushData,
  notificationKindFromPushData,
  notificationTarget,
  notificationTime,
  notificationTitle,
  unreadForSource,
} from "./notificationPresentation";
import { notificationSchema } from "./notificationContracts";

const C = `nyxa-${"a".repeat(32)}`;
const G = `nyxg-${"b".repeat(32)}`;
const T = "7f6b2b3c-1c4a-4c33-9d1e-0a1b2c3d4e5f";
const row = (overrides: Record<string, unknown>) =>
  notificationSchema.parse({
    id: "n",
    kind: "agent_update",
    source: { type: "conversation", id: C },
    created_at: "2026-11-10T08:00:00Z",
    updated_at: "2026-11-10T08:05:00Z",
    ...overrides,
  });

test("badge caps at 9+, count chips start at two and labels read naturally", () => {
  assert.equal(notificationBadge(0), "");
  assert.equal(notificationBadge(-1), "");
  assert.equal(notificationBadge(9), "9");
  assert.equal(notificationBadge(10), "9+");
  assert.equal(notificationBadge(240), "9+");
  assert.equal(notificationCountLabel(1), null);
  assert.equal(notificationCountLabel(3), "3 new");
  assert.equal(notificationBellLabel(0, 0), "Notifications");
  assert.equal(notificationBellLabel(4, 1), "Notifications, 4 unread, 1 needs your decision");
  assert.equal(notificationBellLabel(12, 2), "Notifications, 12 unread, 2 need your decision");
});

test("titles fall back to the source label, then the kind", () => {
  assert.equal(notificationTitle(row({ title: " Report ready " })), "Report ready");
  assert.equal(notificationTitle(row({ source: { type: "trigger", id: T, label: "Morning digest" } })), "Morning digest");
  assert.equal(notificationTitle(row({ kind: "needs_decision" })), "Needs your decision");
  const now = Date.parse("2026-11-10T08:10:00Z");
  assert.equal(notificationTime("2026-11-10T08:05:00Z", now), "5m ago");
  assert.equal(notificationTime("not a date", now), "");
  const label = notificationAccessibilityLabel(
    row({ kind: "needs_decision", title: "Delete repo?", preview: "Allow?", count: 2 }),
    now,
  );
  assert.equal(label, "Delete repo?, Allow?, 5m ago, 2 new, needs your decision, unread");
});

test("taps open the thread when the source names one, else the crew or automation", () => {
  assert.deepEqual(notificationTarget(row({})), { kind: "chat", conversationId: C, agentId: undefined });
  assert.deepEqual(
    notificationTarget(row({ source: { type: "channel_chat", id: C, channel_id: "ch", agent_id: "ag" } })),
    { kind: "chat", conversationId: C, agentId: "ag" },
  );
  assert.deepEqual(
    notificationTarget(row({ source: { type: "trigger", id: T, conversation_id: C } })),
    { kind: "chat", conversationId: C, agentId: undefined },
  );
  assert.deepEqual(notificationTarget(row({ source: { type: "trigger", id: T } })), { kind: "automation", triggerId: T });
  assert.deepEqual(notificationTarget(row({ source: { type: "trigger", id: "legacy" } })), { kind: "automations" });
  assert.deepEqual(notificationTarget(row({ source: { type: "group", id: G } })), { kind: "group", groupId: G });
  assert.deepEqual(notificationTarget(row({ source: { type: "collaboration_group", id: "x", group_id: G } })), {
    kind: "group",
    groupId: G,
  });
  assert.deepEqual(notificationTarget(row({ source: { type: "channel_chat", id: "thread-id" } })), { kind: "none" });
  assert.deepEqual(notificationTarget(row({ source: { type: "group", id: "not-a-group" } })), { kind: "none" });
});

test("a thread owns the summary rows attributed to it, including channel and automation rows", () => {
  const summary = {
    by_source: [
      { source_type: "conversation" as const, id: C, conversation_id: null, unread: 1 },
      { source_type: "channel_chat" as const, id: C, conversation_id: C, unread: 2 },
      { source_type: "trigger" as const, id: T, conversation_id: C, unread: 1 },
      { source_type: "group" as const, id: G, conversation_id: null, unread: 4 },
      { source_type: "collaboration_group" as const, id: G, conversation_id: null, unread: 1 },
    ],
  };
  assert.equal(unreadForSource(summary, "conversation", C), 4);
  assert.equal(unreadForSource(summary, "group", G), 5);
  assert.equal(unreadForSource(summary, "conversation", G), 0);
  assert.equal(unreadForSource(undefined, "conversation", C), 0);
});

test("push data yields only closed kinds and notification ids", () => {
  assert.equal(notificationKindFromPushData({ type: "automation_result" }), "automation_result");
  assert.equal(notificationKindFromPushData({ type: "approval_request" }), null);
  assert.equal(notificationKindFromPushData("automation_result"), null);
  assert.equal(notificationIdFromPushData({ notification_id: " n1 " }), "n1");
  assert.equal(notificationIdFromPushData({ notificationId: "n2" }), "n2");
  assert.equal(notificationIdFromPushData({ request_id: "r" }), null);
});
