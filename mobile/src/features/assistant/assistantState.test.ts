import assert from "node:assert/strict";
import test from "node:test";
import {
  continuationText,
  acknowledgementTitle,
  acknowledgementDecisionLabel,
  mergeAssistantHistory,
  needsClientContinuation,
  retryInput,
} from "./assistantState";
import {
  acknowledgementSchema,
  historySchema,
  messageSchema,
} from "./assistantTypes";

const id = `nyxa-${"a".repeat(32)}`;
const message = (
  seq: number,
  role: "user" | "assistant" = "user",
  text = String(seq),
) =>
  messageSchema.parse({
    id: `m${seq}`,
    seq,
    turn_id: "turn-1",
    role,
    text,
    status: "completed",
    error_code: null,
    created_at: "2026-10-13T00:00:00Z",
  });
function history(sequences: number[], before: number | null = null) {
  return historySchema.parse({
    conversation: {
      id,
      title: "Test",
      model: "nyxagent/chat",
      created_at: "2026-10-13T00:00:00Z",
      last_message_at: "2026-10-13T00:00:00Z",
      message_count: 5,
      active_turn: null,
    },
    messages: sequences.map((seq) => message(seq)),
    before_seq: before,
  });
}
const acknowledgement = acknowledgementSchema.parse({
  id: "d9428888-122b-11e1-b85c-61cd3cbb3210",
  kind: "action",
  status: "allowed",
  summary: "Delete selected file",
  service_name: null,
  service_slug: null,
  tool_name: "delete_file",
  created_at: "2026-10-13T00:00:00Z",
  expires_at: "2026-10-13T01:00:00Z",
});

test("polling keeps earlier loaded pages and replaces updated messages by seq", () => {
  const previous = history([1, 2, 3], 1);
  const page = history([3, 4, 5], 3);
  page.messages[0]!.text = "updated";
  const merged = mergeAssistantHistory(previous, page);
  assert.deepEqual(
    merged.messages.map((item) => item.seq),
    [1, 2, 3, 4, 5],
  );
  assert.equal(merged.messages[2]?.text, "updated");
  assert.equal(merged.before_seq, 1);
  assert.equal(
    mergeAssistantHistory(merged, history([1], null), true).before_seq,
    null,
  );
});

test("conversation switching never merges the previous owner's thread", () => {
  const page = history([9]);
  page.conversation.id = `nyxa-${"b".repeat(32)}`;
  assert.deepEqual(
    mergeAssistantHistory(history([1]), page).messages.map((item) => item.seq),
    [9],
  );
});

test("foreground recovery preserves a cursor for a missing middle after older history reached the start", () => {
  const old = history([1, 2, 3], null);
  const resumed = mergeAssistantHistory(old, history([8, 9, 10], 8));
  assert.equal(resumed.before_seq, 8);
  assert.deepEqual(
    resumed.messages.map((item) => item.seq),
    [1, 2, 3, 8, 9, 10],
  );
  const firstBackfill = mergeAssistantHistory(
    resumed,
    history([5, 6, 7], 5),
    true,
  );
  assert.equal(firstBackfill.before_seq, 5);
  const polled = mergeAssistantHistory(firstBackfill, history([9, 10, 11], 9));
  assert.equal(polled.before_seq, 5);
  const completed = mergeAssistantHistory(
    polled,
    history([1, 2, 3, 4], null),
    true,
  );
  assert.equal(completed.before_seq, null);
  assert.deepEqual(
    completed.messages.map((item) => item.seq),
    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
  );
});

test("account confirmation identifies management versus specialist read access", () => {
  assert.match(
    acknowledgementTitle({ ...acknowledgement, kind: "account" }),
    /manage.*keys, channel bots, services, nodes and approval settings/,
  );
  assert.match(
    acknowledgementTitle({
      ...acknowledgement,
      kind: "account",
      decider: "orchestrator",
    }),
    /read/,
  );
});

test("decision labels mirror Web and never equate a used confirmation with action success", () => {
  assert.equal(
    acknowledgementDecisionLabel({
      ...acknowledgement,
      status: "used",
      decided_by: "user",
    }),
    "Used",
  );
  assert.equal(
    acknowledgementDecisionLabel({ ...acknowledgement, decided_by: "user" }),
    "Allowed by you",
  );
  assert.equal(
    acknowledgementDecisionLabel({
      ...acknowledgement,
      status: "denied",
      decided_by: "orchestrator",
      reason: " Outside the requested scope. ",
    }),
    "Denied by NyxBot: Outside the requested scope.",
  );
  assert.equal(
    acknowledgementDecisionLabel({ ...acknowledgement, status: "expired" }),
    "Expired",
  );
  assert.equal(
    acknowledgementDecisionLabel({
      ...acknowledgement,
      status: "unknown" as never,
    }),
    "Status unavailable",
  );
});

test("retry requires a failed assistant reply and its persisted input, excluding uncertain or stopped effects", () => {
  const data = history([1]);
  const failed = {
    ...message(2, "assistant"),
    status: "failed" as const,
    error_code: "idle_timeout",
  };
  assert.equal(retryInput(data, failed), "1");
  assert.equal(
    retryInput(data, { ...failed, error_code: "outcome_unknown" }),
    undefined,
  );
  assert.equal(
    retryInput(data, { ...failed, error_code: "cancelled" }),
    undefined,
  );
  assert.equal(retryInput(data, { ...failed, status: "completed" }), undefined);
  assert.equal(
    retryInput(data, { ...failed, turn_id: "other-turn" }),
    undefined,
  );
});

test("server-owned specialist, automation, and trigger confirmations never duplicate a continuation", () => {
  assert.equal(needsClientContinuation(acknowledgement), true);
  assert.equal(
    needsClientContinuation({
      ...acknowledgement,
      continuation_owner: "server",
    }),
    false,
  );
  assert.equal(
    needsClientContinuation({ ...acknowledgement, decider: "orchestrator" }),
    false,
  );
  assert.equal(
    needsClientContinuation({ ...acknowledgement, trigger_run_id: "run-1" }),
    false,
  );
  assert.equal(
    needsClientContinuation({ ...acknowledgement, status: "denied" }),
    false,
  );
  assert.match(
    continuationText(acknowledgement),
    /acknowledgement_id d9428888-122b-11e1-b85c-61cd3cbb3210/,
  );
});
