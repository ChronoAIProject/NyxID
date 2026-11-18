import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema } from "../../features/agents/agentContracts";
import { conversationSchema } from "../../features/assistant/assistantTypes";
import {
  DRAWER_PREVIEW_LIMIT,
  boundedDrawerPreview,
  drawerAgentConversations,
  orderedDrawerAgents,
  searchDrawerItems,
  splitChannelThreads,
  threadTitle,
  threadStatus,
} from "./workspaceChatListModel";

const agent = (id: string, overrides: object = {}) =>
  agentSchema.parse({
    id,
    name: "Same name",
    kind: "specialist",
    description: "",
    status: "idle",
    ...overrides,
  });
const chat = (id: number, agentId: string | null, overrides: object = {}) =>
  conversationSchema.parse({
    id: `nyxa-${id.toString(16).padStart(32, "0")}`,
    title: "Same title",
    model: "nyxagent/chat",
    created_at: "2026-10-16T00:00:00Z",
    last_message_at: "2026-10-16T00:00:00Z",
    message_count: 1,
    active_turn: null,
    role: "subagent",
    agent: agentId
      ? { id: agentId, name: "Same name", kind: "specialist" }
      : null,
    ...overrides,
  });

test("specialist direct chats retain their subagent role and group only by opaque ID", () => {
  const first = chat(1, "agent-a");
  const second = chat(2, "agent-b");
  const legacy = chat(3, null);
  assert.deepEqual(
    drawerAgentConversations(
      [{ conversations: [first, second, legacy] }],
      agent("agent-a"),
    ),
    [first],
  );
  assert.deepEqual(
    drawerAgentConversations(
      [{ conversations: [first, second] }],
      agent("agent-b"),
    ),
    [second],
  );
});

test("only the default NyxBot can receive unresolved legacy conversations", () => {
  const legacy = chat(1, null, { role: "orchestrator" });
  const specialist = chat(2, "agent-a");
  assert.deepEqual(
    drawerAgentConversations(
      [{ conversations: [legacy, specialist] }],
      agent("nyxbot", { kind: "nyxbot" }),
    ),
    [legacy],
  );
});

test("overlapping cursor pages deduplicate by ID and retain the most recent row", () => {
  const older = chat(1, "agent-a");
  const newer = {
    ...older,
    title: "Renamed chat",
    last_message_at: "2026-10-16T02:00:00Z",
  };
  const other = chat(2, "agent-a", { last_message_at: "2026-10-16T01:00:00Z" });
  assert.deepEqual(
    drawerAgentConversations(
      [{ conversations: [newer, other] }, { conversations: [older, other] }],
      agent("agent-a"),
    ),
    [newer, other],
  );
});

test("destroyed agents keep their saved chats available", () => {
  const saved = chat(1, "retired", {
    agent: {
      id: "retired",
      name: "Past helper",
      kind: "specialist",
      destroyed: true,
    },
  });
  assert.deepEqual(
    drawerAgentConversations(
      [{ conversations: [saved] }],
      agent("retired", { status: "destroyed" }),
    ),
    [saved],
  );
});

test("NyxBot is pinned first, followed by named live agents and then past agents", () => {
  const rows = [
    agent("z", { display_name: "Zulu" }),
    agent("past", { display_name: "A past agent", status: "destroyed" }),
    agent("bot", { kind: "nyxbot" }),
    agent("a", { display_name: "Alpha" }),
  ];
  assert.deepEqual(
    orderedDrawerAgents(rows).map((row) => row.id),
    ["bot", "a", "z", "past"],
  );
  assert.equal(rows[0]!.id, "z");
});

test("drawer previews cap long lists at five rows in their existing order", () => {
  const rows = Array.from({ length: 8 }, (_, index) => ({ id: String(index) }));
  assert.equal(DRAWER_PREVIEW_LIMIT, 5);
  assert.deepEqual(boundedDrawerPreview(rows), rows.slice(0, 5));
  assert.deepEqual(boundedDrawerPreview(rows.slice(0, 2)), rows.slice(0, 2));
  assert.deepEqual(boundedDrawerPreview([]), []);
});

test("an active row outside the preview replaces its final slot, never becoming a sixth row", () => {
  const rows = Array.from({ length: 8 }, (_, index) => ({ id: String(index) }));
  const preview = boundedDrawerPreview(rows, "7");
  assert.deepEqual(
    preview.map((row) => row.id),
    ["0", "1", "2", "3", "7"],
  );
  assert.equal(preview.length, 5);
  assert.equal(preview[4], rows[7]);
  assert.deepEqual(boundedDrawerPreview(rows, "1"), rows.slice(0, 5));
});

test("a missing active row does not displace preview content or invent a row", () => {
  const rows = Array.from({ length: 8 }, (_, index) => ({ id: String(index) }));
  assert.deepEqual(boundedDrawerPreview(rows, "absent"), rows.slice(0, 5));
  assert.deepEqual(boundedDrawerPreview([], "absent"), []);
});

test("custom preview caps still contain the preferred row and zero has no rows", () => {
  const rows = [{ id: "first" }, { id: "second" }, { id: "third" }];
  assert.deepEqual(boundedDrawerPreview(rows, "third", 2), [rows[0], rows[2]]);
  assert.deepEqual(boundedDrawerPreview(rows, "third", 1), [rows[2]]);
  assert.deepEqual(boundedDrawerPreview(rows, "third", 0), []);
  assert.deepEqual(boundedDrawerPreview(rows, "third", -1), []);
});

test("preview replacement never mutates the original list or its objects", () => {
  const rows = Object.freeze(
    Array.from({ length: 8 }, (_, index) =>
      Object.freeze({ id: String(index), label: `Row ${index}` }),
    ),
  );
  const preview = boundedDrawerPreview(rows, "7");
  assert.equal(rows.length, 8);
  assert.deepEqual(
    rows.map((row) => row.id),
    ["0", "1", "2", "3", "4", "5", "6", "7"],
  );
  assert.notEqual(preview, rows);
  assert.equal(preview[4], rows[7]);
});

test("drawer search normalizes case and whitespace on both query and label metadata", () => {
  const rows = [
    { id: "x", name: "My X\n  Digest", handle: "x-digest" },
    { id: "research", name: "Research Group", handle: "research-team" },
    { id: "other", name: "Other", handle: "other" },
  ];
  const text = (row: (typeof rows)[number]) => `${row.name} ${row.handle}`;
  assert.deepEqual(searchDrawerItems(rows, "  X\t DIGEST  ", text), [rows[0]]);
  assert.deepEqual(searchDrawerItems(rows, "search group", text), [rows[1]]);
  assert.deepEqual(searchDrawerItems(rows, "RESEARCH-TEAM", text), [rows[1]]);
  assert.deepEqual(searchDrawerItems(rows, "does not exist", text), []);
  // Search is an ordered substring, rather than an unordered bag of words.
  assert.deepEqual(searchDrawerItems(rows, "digest my", text), []);
});

test("empty searches return every row in order and search never mutates its input", () => {
  const rows = Object.freeze([
    Object.freeze({ id: "a", label: "Alpha" }),
    Object.freeze({ id: "b", label: "Beta" }),
  ]);
  const text = (row: (typeof rows)[number]) => row.label;
  const all = searchDrawerItems(rows, " \n\t ", text);
  assert.deepEqual(all, rows);
  assert.notEqual(all, rows);
  assert.deepEqual(searchDrawerItems(rows, "A", text), rows);
  assert.deepEqual(searchDrawerItems([], "alpha", text), []);
  assert.deepEqual(
    rows.map((row) => row.id),
    ["a", "b"],
  );
});

test("channel metadata retains separate bot identities and parent/topic labels in newest-first order", () => {
  const channel = {
    platform: "telegram",
    channel_agent_id: "bot-one",
    bot_label: "Same bot name",
    chat_id: "chat",
    chat_title: "Customer chat",
    parent_chat_id: "parent",
    parent_title: "Support",
    thread_id: "topic",
  };
  const direct = chat(1, "agent-a");
  const oldest = chat(2, "agent-a", { channel });
  const latest = chat(3, "agent-a", {
    channel,
    last_message_at: "2026-10-17T00:00:00Z",
  });
  const otherBot = chat(4, "agent-a", {
    channel: { ...channel, channel_agent_id: "bot-two" },
  });
  const legacy = chat(5, "agent-a", { channel: { platform: "lark" } });
  const split = splitChannelThreads(
    drawerAgentConversations(
      [{ conversations: [oldest, otherBot, latest, direct, legacy] }],
      agent("agent-a"),
    ),
  );
  assert.deepEqual(split.own, [direct]);
  assert.deepEqual(
    split.bots.map((bot) => bot.key),
    ["bot-one", "bot-two", "platform:lark"],
  );
  assert.deepEqual(split.bots[0]!.threads, [latest, oldest]);
  assert.equal(split.bots[2]!.label, "Lark bot");
  assert.equal(split.bots[0]!.threads[0]!.channel?.parent_title, "Support");
  assert.equal(threadTitle(latest), "Customer chat");
  assert.equal(threadTitle(legacy), "Same title");
});

test("pending decisions take precedence over running turns without inventing completed work", () => {
  const idle = chat(1, "a");
  const running = chat(2, "a", {
    active_turn: { turn_id: "turn", started_at: "2026-10-17T00:00:00Z" },
  });
  assert.equal(threadStatus(idle), undefined);
  assert.equal(threadStatus(running), "Running");
  assert.equal(
    threadStatus({ ...running, pending_acknowledgements: 2 }),
    "Needs you",
  );
});
