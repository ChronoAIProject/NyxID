import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import assert from "node:assert/strict";
import { test } from "vitest";
import { readFileSync } from "node:fs";
import { nyxAgentHistorySchema as historySchema, nyxAgentAcknowledgementSchema as acknowledgementSchema } from "./assistant-nyxagent";
import { KNOWN_TOOL_FAILURE_KINDS, activityPresentationStatus, parseHistoryResponse, HISTORY_UNAVAILABLE, historyErrorMessage } from "./assistant-history-compat";

const sourceDirectory = dirname(fileURLToPath(import.meta.url));
const fixture = JSON.parse(readFileSync(resolve(sourceDirectory, "../lib/assistant/__fixtures__/retry-duplicate-history.json"), "utf8"));
const known = ["service_status", "tool_error", "refused", "internal", "resource_unavailable", "retry_duplicate_action"];

test("2345 audited failure mapping matches the documented producers and the other client", () => {
  assert.deepEqual([...KNOWN_TOOL_FAILURE_KINDS], known);
  const doc = readFileSync(resolve(sourceDirectory, "../../../docs/chat/activity-history-compat.md"), "utf8");
  for (const kind of known) assert.ok(doc.includes("`" + kind + "`"));
  const other = readFileSync(resolve(sourceDirectory, "../../../mobile/src/features/assistant/assistantHistoryCompat.ts"), "utf8");
  for (const kind of known) assert.ok(other.includes('"' + kind + '"'));
});

test("2345 exact reported activity fragment accepts all current and future failure kinds", () => {
  for (const kind of [...known, "future_failure_kind", ""]) {
    const wire = structuredClone(fixture);
    wire.conversation.active_turn.activities[1].failure.kind = kind;
    const page = historySchema.parse(wire);
    assert.equal(page.conversation.active_turn?.activities[1]?.failure?.kind, kind);
    assert.equal(page.conversation.active_turn?.activities.length, 13);
  }
  assert.equal(activityPresentationStatus("completed", "retry_duplicate_action"), "skipped");
  assert.equal(activityPresentationStatus("completed", "future_failure_kind"), "unknown");
  assert.equal(activityPresentationStatus("completed", ""), "unknown");
  assert.equal(activityPresentationStatus("error", ""), "error");
  assert.equal(activityPresentationStatus("completed", null), "completed");
});

test("2345 future display metadata does not invalidate the conversation or invent success", () => {
  const wire = structuredClone(fixture);
  wire.conversation.role = "future_role";
  wire.conversation.title_source = "future_source";
  wire.conversation.agent = { id: "agent", name: "Agent", kind: "future_kind", destroyed: true };
  wire.conversation.active_turn.activities[0].status = "future_status";
  wire.messages[0].role = "future_role";
  wire.messages[0].status = "future_status";
  wire.messages[0].feedback = { vote: "future_vote" };
  wire.messages[0].recovery = { status: "future_recovery", reason: "new", text: null, attachments: [] };
  wire.messages[0].steering = { outcome: "future_outcome", client_request_id: "r", code: null };
  const page = historySchema.parse(wire);
  assert.equal(page.conversation.agent?.destroyed, true);
  assert.equal(page.conversation.agent?.kind, "unknown");
  assert.equal(page.conversation.active_turn?.activities[0]?.status, "unknown");
  assert.equal(page.messages[0]?.role, "unknown");
  assert.equal(page.messages[0]?.status, "unknown");
  assert.equal(page.messages[0]?.recovery?.status, "unknown");
  assert.notEqual(page.messages[0]?.steering?.outcome, "applied");
  assert.notEqual(page.messages[0]?.feedback?.vote, "up");
});

test("2345 unsupported authority cards are isolated, announced, and never made actionable", () => {
  const wire = { ...fixture, acknowledgements: [{ id: "new", kind: "future_authority", status: "allowed" }], approvals: [{ status: "future_approval" }] };
  const page = historySchema.parse(wire);
  assert.deepEqual(page.acknowledgements, []);
  assert.deepEqual(page.approvals, []);
  assert.equal(page.unsupported_history_items, 2);
  assert.equal(historySchema.parse({ ...fixture, unsupported_history_items: 99 }).unsupported_history_items, undefined);
  assert.equal(acknowledgementSchema.safeParse(wire.acknowledgements[0]).success, false);
});

test("2345 malformed required history is a friendly fixed error, not validation JSON", () => {
  for (const wire of [{ ...fixture, messages: [{ text: "private" }] }, { ...fixture, conversation: { ...fixture.conversation, active_turn: false } }]) {
    assert.throws(() => parseHistoryResponse(historySchema, wire), { message: HISTORY_UNAVAILABLE });
    const result = historySchema.safeParse(wire);
    assert.equal(result.success, false);
    if (!result.success) assert.equal(historyErrorMessage(result.error), HISTORY_UNAVAILABLE);
  }
});

test("2345 future attachment, machine, platform and delegation metadata stay local", () => {
  const wire = structuredClone(fixture);
  wire.conversation.platform_thread = { platform: "future_platform", manager_id: "bot", binding_id: "11111111-1111-4111-8111-111111111111", binding_generation: 1 };
  wire.conversation.active_turn.activities[0].machine = {
    operation_id: "op", node_id: "node", agent_id: "agent", context_mode: "future_context", action: "file.read", status: "future_status",
    job_id: null, exit_code: null, bytes: null, duration_ms: null, error_code: null, screenshot_id: null, preview_id: null, preview_enabled: false,
  };
  wire.messages[0].attachments = [{ id: "image", content_type: "image/png", label: "Image", size: 1, image_input: "future_delivery" }];
  wire.cma_operations = [{ id: "cma", turn_id: "turn", kind: "future_kind", operation_id: null, agent_id: null, idempotency_key: null, status: null, phase: null, state: "future_state", reason: null, monitor_name: "Agent", created_at: "2026-10-28T01:00:00Z", checked_at: null }];
  const page = historySchema.parse(wire);
  assert.equal(page.conversation.active_turn?.activities[0]?.machine?.context_mode, "unknown");
  assert.notEqual(page.messages[0]?.attachments[0]?.image_input, "sent");
  assert.equal(page.cma_operations?.[0]?.kind, "future_kind");
});
