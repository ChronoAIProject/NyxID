import assert from "node:assert/strict";
import { test } from "node:test";
import type { AssistantLiveEvent } from "../../lib/api/assistantLiveTransport";
import { groupKeys, groupLiveRefresh } from "./groupLiveRefresh";

const PERSONAL = "nyxg-0123456789abcdef0123456789abcdef";
const ORGANIZATION = "nyxg-1123456789abcdef0123456789abcdef";
type Conversation = Extract<AssistantLiveEvent, { type: "conversation" }>;
const conversation = (overrides: Partial<Conversation> = {}): Conversation => ({
  type: "conversation",
  id: "nyxa-0123456789abcdef0123456789abcdef",
  group_id: PERSONAL,
  turn_id: "turn-1",
  messages: 2,
  ...overrides,
});

test("personal and organization group notifications refresh only the exact epoch group and group list", () => {
  const seen = new Map<string, string>();
  for (const id of [PERSONAL, ORGANIZATION]) {
    assert.deepEqual(groupLiveRefresh(4, { type: "group", id }, seen), {
      now: [["workspace", "groups", 4, "messages", id]],
      lists: [["workspace", "groups", 4, "list"]],
    });
  }
  assert.equal(seen.size, 0);
});

test("hidden group-member conversations refresh their group, never a private thread", () => {
  const seen = new Map<string, string>();
  assert.deepEqual(groupLiveRefresh(4, conversation(), seen), {
    now: [["workspace", "groups", 4, "messages", PERSONAL]],
    lists: [
      ["workspace", "groups", 4, "list"],
      ["workspace", "agents"],
    ],
  });
  assert.equal(seen.size, 1);
});

test("repeated metadata refreshes only the transcript while turn, count or title changes refresh lists", () => {
  const seen = new Map<string, string>();
  const event = conversation();
  groupLiveRefresh(4, event, seen);
  const transcriptOnly = {
    now: [["workspace", "groups", 4, "messages", PERSONAL]],
    lists: [],
  };
  assert.deepEqual(groupLiveRefresh(4, { ...event }, seen), transcriptOnly);
  for (const next of [
    conversation({ messages: 3 }),
    conversation({ messages: 3, turn_id: "turn-2" }),
    conversation({ messages: 3, turn_id: null }),
    conversation({ messages: 3, turn_id: null, title_changed: true }),
  ]) {
    assert.deepEqual(groupLiveRefresh(4, next, seen).lists, [
      ["workspace", "groups", 4, "list"],
      ["workspace", "agents"],
    ]);
  }
  assert.deepEqual(
    groupLiveRefresh(4, conversation({ messages: 3, turn_id: null }), seen),
    transcriptOnly,
  );
});

test("private and channel notifications refresh drawer metadata without entering crew transcripts", () => {
  const seen = new Map<string, string>();
  const event = conversation({ group_id: null });
  const refresh = {
    now: [],
    lists: [
      ["workspace", "drawer-chats", 4],
      ["workspace", "agents"],
    ],
  };
  assert.deepEqual(groupLiveRefresh(4, event, seen), refresh);
  assert.equal(seen.size, 1);
  assert.deepEqual(groupLiveRefresh(4, event, seen), { now: [], lists: [] });
  assert.deepEqual(
    groupLiveRefresh(4, { ...event, turn_id: null }, seen),
    refresh,
  );
  for (const event of [
    { type: "channels" as const },
    {
      type: "channel_thread" as const,
      id: "child",
      channel_id: "bot",
      parent_id: "parent",
      conversation_id: "private",
    },
  ])
    assert.deepEqual(groupLiveRefresh(4, event, seen), refresh);
});

test("ready and resync clear signatures and refresh the epoch group root plus agents", () => {
  for (const type of ["ready", "resync"] as const) {
    const seen = new Map<string, string>();
    groupLiveRefresh(4, conversation(), seen);
    assert.deepEqual(groupLiveRefresh(4, { type }, seen), {
      now: [
        ["workspace", "groups", 4],
        ["workspace", "agents"],
        ["workspace", "drawer-chats", 4],
      ],
      lists: [],
    });
    assert.equal(seen.size, 0);
    assert.equal(groupLiveRefresh(4, conversation(), seen).lists.length, 2);
  }
});

test("actor epochs have distinct query roots and never reuse another actor's signature", () => {
  assert.notDeepEqual(groupKeys(4), groupKeys(5));
  const seen = new Map<string, string>();
  groupLiveRefresh(4, conversation(), seen);
  assert.deepEqual(groupLiveRefresh(5, conversation(), seen), {
    now: [["workspace", "groups", 5, "messages", PERSONAL]],
    lists: [
      ["workspace", "groups", 5, "list"],
      ["workspace", "agents"],
    ],
  });
  assert.equal(seen.size, 2);
  assert.deepEqual(groupLiveRefresh(4, conversation(), seen).lists, []);
});

test("seen metadata is bounded to 256 FIFO threads and repeating a thread does not renew it", () => {
  const seen = new Map<string, string>();
  for (let index = 0; index < 256; index++)
    groupLiveRefresh(4, conversation({ id: `thread-${index}` }), seen);
  assert.equal(seen.size, 256);
  assert.deepEqual(
    groupLiveRefresh(4, conversation({ id: "thread-0" }), seen).lists,
    [],
  );
  groupLiveRefresh(4, conversation({ id: "thread-256" }), seen);
  assert.equal(seen.size, 256);
  assert.equal(seen.has("4:thread-0"), false);
  assert.equal(seen.has("4:thread-1"), true);
  assert.equal(seen.has("4:thread-256"), true);
  // An evicted thread is conservatively treated as changed if it returns.
  assert.equal(
    groupLiveRefresh(4, conversation({ id: "thread-0" }), seen).lists.length,
    2,
  );
  assert.equal(seen.size, 256);
  assert.equal(seen.has("4:thread-1"), false);
});
