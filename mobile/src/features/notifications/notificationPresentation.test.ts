import assert from "node:assert/strict";
import test from "node:test";
import {
  EMPTY_NOTIFICATION_MARKERS,
  notificationAccessibilityLabel,
  notificationBadge,
  notificationBellLabel,
  notificationCountLabel,
  notificationIdFromPushData,
  notificationKindFromPushData,
  notificationLinkTarget,
  notificationMarkers,
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

test("badge caps at 99+, count chips start at two and labels read naturally", () => {
  assert.equal(notificationBadge(0), "");
  assert.equal(notificationBadge(-1), "");
  assert.equal(notificationBadge(9), "9");
  assert.equal(notificationBadge(10), "10");
  assert.equal(notificationBadge(99), "99");
  assert.equal(notificationBadge(100), "99+");
  assert.equal(notificationBadge(240), "99+");
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

const ORIGIN = "https://app.nyxid.test";
const ORIGINS = [`${ORIGIN}/`, "https://app.nyxid.dev"];
const AGENT = "7c0f6a8e-2b2e-4a1a-9f1f-0d2c5a6b7c8d";

test("canonical links on a first-party origin select the exact page; other origins, fragments and unknown pages yield nothing", () => {
  assert.deepEqual(notificationLinkTarget(`${ORIGIN}/assistant?c=${C}`, ORIGINS), { kind: "chat", conversationId: C });
  assert.deepEqual(notificationLinkTarget(`${ORIGIN}/assistant?c=${C}&card=card-1`, ORIGINS), {
    kind: "chat",
    conversationId: C,
    acknowledgementId: "card-1",
  });
  assert.deepEqual(notificationLinkTarget(`https://app.nyxid.dev/assistant?c=${G}`, ORIGINS), { kind: "group", groupId: G });
  assert.deepEqual(notificationLinkTarget(`/assistant?g=${G}`, ORIGINS), { kind: "group", groupId: G });
  assert.deepEqual(notificationLinkTarget(`${ORIGIN}/assistant?agent=${AGENT}&details=channels`, ORIGINS), {
    kind: "agent",
    agentId: AGENT,
  });
  assert.deepEqual(notificationLinkTarget(`${ORIGIN}/assistant?agent=${AGENT}`, ORIGINS), {
    kind: "chat",
    agentId: AGENT,
    openLatest: true,
  });
  assert.deepEqual(notificationLinkTarget(`${ORIGIN}/assistant/automations?agent=${AGENT}`, ORIGINS), {
    kind: "automations",
    agentId: AGENT,
  });
  assert.deepEqual(notificationLinkTarget(`${ORIGIN}/assistant/automations`, ORIGINS), { kind: "automations" });
  for (const [href, why] of [
    [`https://evil.example/assistant?c=${C}`, "another origin"],
    [`https://app.nyxid.test.evil.example/assistant?c=${C}`, "a lookalike host"],
    [`http://app.nyxid.test/assistant?c=${C}`, "a different scheme on the same host"],
    [`${ORIGIN}/assistant?c=${C}#frag`, "a fragment"],
    [`https://user:pw@app.nyxid.test/assistant?c=${C}`, "credentials"],
    [`${ORIGIN}/assistant?c=not-a-thread`, "a malformed thread id"],
    [`${ORIGIN}/assistant?g=bad`, "a malformed crew id"],
    [`${ORIGIN}/assistant?agent=bad%20agent&details=channels`, "a malformed agent id"],
    [`${ORIGIN}/assistant?panel=billing`, "an account panel"],
    [`${ORIGIN}/assistant`, "the bare workspace"],
    [`${ORIGIN}/keys?tab=services`, "an unrelated page"],
    ["nyxid://assistant?c=" + C, "the app scheme is not a frontend link"],
    ["not a url", "junk"],
  ] as const)
    assert.equal(notificationLinkTarget(href, ORIGINS), null, why);
  assert.equal(notificationLinkTarget(`${ORIGIN}/assistant?c=${C}`, []), null, "no configured origin trusts nothing");
  assert.equal(notificationLinkTarget(`${ORIGIN}/assistant?c=${C}`, ["", "not-a-url"]), null);
  assert.deepEqual(notificationLinkTarget(`/assistant?c=${C}`, ["", ORIGIN]), { kind: "chat", conversationId: C }, "relative links resolve against the first valid origin");
  assert.equal(notificationLinkTarget(null, ORIGINS), null);
});

test("the canonical link decides the destination, the references remain the fallback and refine automation links", () => {
  const guestTranscript = row({
    kind: "chat_update",
    source: { type: "channel_chat", id: "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", channel_id: "ch", agent_id: AGENT },
    url: `${ORIGIN}/assistant?agent=${AGENT}&details=channels`,
  });
  assert.deepEqual(notificationTarget(guestTranscript, ORIGINS), { kind: "agent", agentId: AGENT });
  assert.deepEqual(notificationTarget(guestTranscript, ["https://other.example"]), { kind: "none" }, "an unreadable link never exposes the hidden thread id");
  assert.deepEqual(notificationTarget(guestTranscript), { kind: "none" });
  const crossOrigin = row({ url: `https://evil.example/assistant?c=${G}` });
  assert.deepEqual(notificationTarget(crossOrigin, ORIGINS), { kind: "chat", conversationId: C, agentId: undefined }, "a foreign link falls back to the row's own thread");
  const card = row({ kind: "needs_decision", source: { type: "conversation", id: C, agent_id: "ag" }, url: `${ORIGIN}/assistant?c=${C}&card=card-9` });
  assert.deepEqual(notificationTarget(card, ORIGINS), { kind: "chat", conversationId: C, acknowledgementId: "card-9", agentId: "ag" });
  const crew = row({ kind: "group_mention", source: { type: "group", id: G }, url: `${ORIGIN}/assistant?c=${G}` });
  assert.deepEqual(notificationTarget(crew, ORIGINS), { kind: "group", groupId: G });
  const run = row({ kind: "automation_result", source: { type: "trigger", id: T, agent_id: AGENT }, url: `${ORIGIN}/assistant/automations?agent=${AGENT}` });
  assert.deepEqual(notificationTarget(run, ORIGINS), { kind: "automation", triggerId: T }, "a generic automations link opens the run's own trigger");
  const legacyRun = row({ kind: "automation_result", source: { type: "trigger", id: "legacy", agent_id: AGENT }, url: `${ORIGIN}/assistant/automations?agent=${AGENT}` });
  assert.deepEqual(notificationTarget(legacyRun, ORIGINS), { kind: "automations", agentId: AGENT });
  const threadRun = row({ kind: "automation_result", source: { type: "trigger", id: T, conversation_id: C }, url: `${ORIGIN}/assistant?c=${C}` });
  assert.deepEqual(notificationTarget(threadRun, ORIGINS), { kind: "chat", conversationId: C });
});

test("sidebar markers come from one pass over the summary and match the per-source reads", () => {
  const summary = {
    by_source: [
      { source_type: "conversation" as const, id: C, conversation_id: null, unread: 1 },
      { source_type: "channel_chat" as const, id: C, conversation_id: C, unread: 2 },
      { source_type: "trigger" as const, id: T, conversation_id: C, unread: 1 },
      { source_type: "channel_chat" as const, id: "hidden", conversation_id: null, unread: 3 },
      { source_type: "group" as const, id: G, conversation_id: null, unread: 4 },
      { source_type: "collaboration_group" as const, id: G, conversation_id: null, unread: 1 },
      { source_type: "conversation" as const, id: "zero", conversation_id: null, unread: 0 },
    ],
  };
  assert.deepEqual(notificationMarkers(summary), { conversations: { [C]: 4 }, groups: { [G]: 5 } });
  assert.equal(notificationMarkers(undefined), EMPTY_NOTIFICATION_MARKERS);
  assert.equal(notificationMarkers({ by_source: [] }), EMPTY_NOTIFICATION_MARKERS);
  assert.equal(unreadForSource(summary, "conversation", C), notificationMarkers(summary).conversations[C]);
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
