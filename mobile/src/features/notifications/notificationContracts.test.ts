import assert from "node:assert/strict";
import test from "node:test";
import {
  DEFAULT_NOTIFICATION_PUSH,
  MAX_NOTIFICATION_READ_IDS,
  NOTIFICATION_KINDS,
  isNotificationKind,
  notificationFilterKeeps,
  notificationFilterKinds,
  notificationPageSchema,
  notificationPushPreferences,
  notificationReadBody,
  notificationSummarySchema,
} from "./notificationContracts";

const item = {
  id: "n1",
  kind: "chat_update",
  source: { type: "channel_chat", id: `nyxa-${"a".repeat(32)}`, channel_id: "ch" },
  created_at: "2026-11-10T08:00:00Z",
  updated_at: "2026-11-10T08:05:00Z",
};

test("feed pages apply additive defaults and keep the mark-all fence", () => {
  const page = notificationPageSchema.parse({
    items: [item],
    as_of: "2026-11-10T08:06:00Z",
  });
  assert.equal(page.next_cursor, null);
  const row = page.items[0]!;
  assert.equal(row.count, 1);
  assert.equal(row.read, false);
  assert.equal(row.resolved, false);
  assert.equal(row.title, "");
  assert.equal(row.url, null);
  assert.equal(row.source.conversation_id, null);
  assert.equal(row.source.channel_id, "ch");
  // A newer server's kind or a reshaped row is dropped, not the whole page,
  // so a rolling upgrade never blanks the feed (web does the same).
  const skewed = notificationPageSchema.parse({
    items: [{ ...item, kind: "surprise" }, item, { ...item, id: "", kind: "chat_update" }],
    as_of: "2026-11-10T08:06:00Z",
  });
  assert.deepEqual(skewed.items.map((entry) => entry.id), [item.id]);
  assert.throws(() => notificationPageSchema.parse({ items: [item] }));
  assert.throws(() => notificationPageSchema.parse({ items: "rows", as_of: "2026-11-10T08:06:00Z" }));
});

test("summary entries carry their thread and the truncation flag", () => {
  const summary = notificationSummarySchema.parse({
    unread_count: 3,
    needs_decision_count: 1,
    by_source: [
      { source_type: "trigger", id: "t", conversation_id: `nyxa-${"b".repeat(32)}`, unread: 2 },
      { source_type: "group", id: `nyxg-${"c".repeat(32)}`, unread: 1 },
    ],
  });
  assert.equal(summary.by_source[0]!.conversation_id, `nyxa-${"b".repeat(32)}`);
  assert.equal(summary.by_source[1]!.conversation_id, null);
  assert.equal(summary.by_source_truncated, false);
  assert.deepEqual(notificationSummarySchema.parse({}), {
    unread_count: 0,
    needs_decision_count: 0,
    by_source: [],
    by_source_truncated: false,
  });
});

test("push preferences default every kind off and tolerate older servers", () => {
  assert.deepEqual(notificationPushPreferences({ access_mode: "nyxmode" }), DEFAULT_NOTIFICATION_PUSH);
  assert.deepEqual(notificationPushPreferences({ notifications: null }), DEFAULT_NOTIFICATION_PUSH);
  assert.deepEqual(
    notificationPushPreferences({ notifications: { push: { needs_decision: true } } }),
    { ...DEFAULT_NOTIFICATION_PUSH, needs_decision: true },
  );
  assert.equal(Object.keys(DEFAULT_NOTIFICATION_PUSH).length, NOTIFICATION_KINDS.length);
  assert.ok(isNotificationKind("group_mention"));
  assert.ok(!isNotificationKind("approval_request"));
});

test("read bodies name exactly one scope and bound ids", () => {
  assert.deepEqual(notificationReadBody({ ids: ["a", "a", "b", ""] }), { ids: ["a", "b"] });
  assert.deepEqual(notificationReadBody({ source: { type: "conversation", id: "c" } }), {
    source: { type: "conversation", id: "c" },
  });
  assert.deepEqual(notificationReadBody({ all: true, before: "2026-11-10T08:06:00Z" }), {
    all: true,
    before: "2026-11-10T08:06:00Z",
  });
  assert.throws(() => notificationReadBody({ ids: [] }));
  assert.throws(() =>
    notificationReadBody({ ids: Array.from({ length: MAX_NOTIFICATION_READ_IDS + 1 }, (_, i) => `id${i}`) }),
  );
  assert.throws(() => notificationReadBody({ source: { type: "conversation", id: "" } }));
  assert.throws(() => notificationReadBody({ all: true, before: "yesterday" }));
  assert.throws(() => notificationReadBody({ source: { type: "card" as never, id: "x" } }));
});

test("filters map to closed kind subsets and Needs you hides resolved cards", () => {
  assert.equal(notificationFilterKinds("all"), undefined);
  assert.deepEqual(notificationFilterKinds("needs_you"), ["needs_decision"]);
  assert.deepEqual(notificationFilterKinds("automations"), ["automation_result", "automation_problem"]);
  assert.deepEqual(notificationFilterKinds("chats"), ["chat_update", "agent_update", "group_mention"]);
  assert.ok(notificationFilterKeeps("needs_you", { kind: "needs_decision", resolved: false }));
  assert.ok(!notificationFilterKeeps("needs_you", { kind: "needs_decision", resolved: true }));
  assert.ok(!notificationFilterKeeps("needs_you", { kind: "chat_update", resolved: false }));
  assert.ok(notificationFilterKeeps("all", { kind: "needs_decision", resolved: true }));
  assert.ok(!notificationFilterKeeps("chats", { kind: "automation_result", resolved: false }));
});

test("actor, platform, bounded crew identities and per-kind counts survive contract parsing", () => {
  const actor = { type: "agent", id: "researcher", name: "Researcher", kind: "specialist", display_name: "Researcher" };
  const page = notificationPageSchema.parse({ items: [{ ...item, actor, participants: [actor], platform: "lark", outcome: "failed", reason: "result_delivery_failed" }], as_of: "2026-11-10T08:06:00Z" });
  assert.equal(page.items[0]!.actor?.kind, "specialist");
  assert.equal(page.items[0]!.participants?.length, 1);
  assert.equal(page.items[0]!.platform, "lark");
  assert.equal(page.items[0]!.reason, "result_delivery_failed");
  assert.equal(notificationSummarySchema.parse({ by_kind: { chat_update: 7 } }).by_kind?.chat_update, 7);
  assert.equal(notificationPageSchema.parse({ items: [{ ...item, participants: [actor, actor, actor, actor] }], as_of: "2026-11-10T08:06:00Z" }).items.length, 0);
});
