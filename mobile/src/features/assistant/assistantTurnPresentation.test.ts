import assert from "node:assert/strict";
import test from "node:test";
import {
  activitySchema,
  conversationSchema,
  messageSchema,
  turnEventSchema,
} from "./assistantTypes";
import { applyAssistantEvent, emptyLiveReply } from "./assistantStream";
import {
  billingError,
  contextResetCopy,
  deriveTurnStatus,
  diagnosticId,
  deliveryText,
  elapsedLabel,
  recoveryReason,
  steeringNotice,
  storedError,
  toolFailureCopy,
} from "./assistantTurnPresentation";

const at = "2026-10-28T01:00:00Z";
const cid = `nyxa-${"a".repeat(32)}`;
export const failedMessage = () =>
  messageSchema.parse({
    id: "reply",
    seq: 2,
    turn_id: "turn-1",
    role: "assistant",
    text: "Partial result",
    status: "failed",
    error_code: "model_interrupted",
    created_at: at,
    upstream_error_code: "upstream_closed",
    activity_total: 42,
    error_detail: {
      stage: "stream",
      http_status: 502,
      upstream_request_id: "req-1",
      response_id: "resp-1",
      continuations: 2,
      turn_id: "turn-1",
      occurred_at: at,
      attempts: 3,
      capacity_wait_ms: 61000,
    },
    turn_metadata: {
      attempts: 3,
      capacity_wait_ms: 61000,
      failure_reason: {
        source: "nyxid",
        stage: "admission",
        code: "route",
        http_status: 502,
        nyxid_error_code: 12103,
      },
      channel_delivery: { status: "failed", heard_prefix_matches_sent: false },
    },
    recovery: {
      status: "review",
      reason: "activity_outcome_uncertain",
      text: "Original request",
      attachments: ["file-id"],
    },
    activities: [
      {
        id: "tool-1",
        label: "api-github__get_issue",
        status: "error",
        started_at: at,
        ended_at: "2026-10-28T01:01:03Z",
        failure: { kind: "service_status", status: 503 },
      },
    ],
  });

test("NyxAgent history retains failure diagnostics, recovery, continuation, delivery and activity bounds", () => {
  const m = failedMessage();
  assert.equal(m.activity_total, 42);
  assert.equal(m.activities[0]?.failure?.status, 503);
  assert.equal(m.error_detail?.upstream_request_id, "req-1");
  assert.equal(m.turn_metadata?.failure_reason?.nyxid_error_code, 12103);
  assert.equal(m.recovery?.reason, "activity_outcome_uncertain");
  assert.equal(
    messageSchema.parse({ ...m, continuation_of: "ack-1" }).continuation_of,
    "ack-1",
  );
  assert.throws(() =>
    activitySchema.parse({ ...m.activities[0], failure: { kind: "invented" } }),
  );
});

test("resumed active turns retain reply checkpoints and capacity timing", () => {
  const c = conversationSchema.parse({
    id: cid,
    title: "T",
    model: "M",
    created_at: at,
    last_message_at: at,
    message_count: 1,
    active_turn: {
      turn_id: "turn-1",
      started_at: at,
      queued: true,
      waiting_for_capacity: true,
      activity_total: 100,
      reply: {
        message_id: "m",
        block_id: "b",
        cursor: 7,
        text: "Resumed partial",
      },
      capacity_wait_started_at: at,
      capacity_wait_deadline: "2026-10-28T01:02:00Z",
    },
  });
  assert.equal(c.active_turn?.reply?.text, "Resumed partial");
  assert.equal(c.active_turn?.queued, true);
  assert.equal(c.active_turn?.activity_total, 100);
});

test("SSE carries capacity, continuation, retry notices and terminal diagnostics through the real reducer", () => {
  let state = applyAssistantEvent(
    emptyLiveReply(),
    turnEventSchema.parse({
      event: "turn.status",
      cursor: 1,
      conversation_id: cid,
      turn_id: "turn-1",
      status: "waiting",
      started_at: at,
      waiting_for_capacity: true,
      queued: true,
    }),
  );
  state = applyAssistantEvent(state, {
    event: "turn.continuing",
    cursor: 2,
    turn_id: "turn-1",
    continuation: 3,
  });
  state = applyAssistantEvent(state, {
    event: "turn.notice",
    cursor: 3,
    code: "retrying",
    message: "Retrying",
  });
  state = applyAssistantEvent(
    state,
    turnEventSchema.parse({
      event: "turn.completed",
      cursor: 4,
      turn_id: "turn-1",
      status: "failed",
      turn_metadata: failedMessage().turn_metadata,
      error: {
        code: "internal_error",
        message: "failed",
        stage: "model",
        upstream_request_id: "req-1",
      },
    }),
  );
  assert.equal(state.status?.waiting_for_capacity, true);
  assert.equal(state.continuations, 3);
  assert.equal(state.noticeCode, "retrying");
  assert.equal(state.result?.error?.upstream_request_id, "req-1");
});

test("turn phases follow web precedence: approval, tool, retrying, continuing, live, active", () => {
  const base = {
    activeTurn: {
      started_at: at,
      continuations: 2,
      activities: [{ label: "tool", status: "running", started_at: at }],
    },
    live: true,
    pendingApproval: true,
    retrying: true,
    interrupted: true,
  };
  assert.equal(deriveTurnStatus(base)?.phase, "approval");
  assert.equal(
    deriveTurnStatus({ ...base, pendingApproval: false })?.phase,
    "tool",
  );
  const idle = {
    ...base,
    pendingApproval: false,
    activeTurn: { started_at: at, continuations: 2 },
  };
  assert.equal(deriveTurnStatus(idle)?.phase, "retrying");
  assert.equal(
    deriveTurnStatus({ ...idle, retrying: false })?.phase,
    "continuing",
  );
  assert.equal(
    deriveTurnStatus({
      ...idle,
      retrying: false,
      activeTurn: { started_at: at },
    })?.phase,
    "working",
  );
  assert.equal(
    deriveTurnStatus({
      ...idle,
      retrying: false,
      live: false,
      activeTurn: { started_at: at },
    })?.label,
    "Working — the reply appears when it is ready",
  );
  assert.equal(deriveTurnStatus({ ...base, activeTurn: null }), null);
});

test("elapsed time hides the first two seconds, clamps negative time and formats minutes", () => {
  assert.equal(elapsedLabel(at, Date.parse(at) + 2999), undefined);
  assert.equal(elapsedLabel(at, Date.parse(at) + 3000), "3s");
  assert.equal(elapsedLabel(at, Date.parse(at) + 63000), "1:03");
  assert.equal(elapsedLabel(at, Date.parse(at) - 1000), undefined);
  assert.equal(elapsedLabel("invalid", 1), undefined);
});

test("all tool failures and channel delivery outcomes have precise web copy", () => {
  assert.equal(
    toolFailureCopy({ kind: "service_status", status: 503 }),
    "The service responded with HTTP 503.",
  );
  assert.match(toolFailureCopy({ kind: "service_status" }), /No HTTP status/);
  assert.equal(
    toolFailureCopy({ kind: "refused" }),
    "NyxID refused this call.",
  );
  assert.equal(
    toolFailureCopy({ kind: "internal" }),
    "NyxID could not complete this call.",
  );
  assert.equal(
    toolFailureCopy({ kind: "tool_error" }),
    "The tool reported an error.",
  );
  assert.match(toolFailureCopy(null), /No additional error details/);
  assert.equal(
    deliveryText({ status: "failed", heard_prefix_matches_sent: false }),
    "Provider reported delivery failure.",
  );
  assert.match(
    deliveryText({
      status: "interrupted_prefix",
      heard_prefix_matches_sent: true,
      heard_prefix_chars: 23,
    }),
    /verified prefix of 23 characters/,
  );
  assert.match(
    deliveryText({ status: "unknown", heard_prefix_matches_sent: false }),
    /unconfirmed/,
  );
});

test("diagnostic copy contains only supplied metadata and web recovery copy explains uncertainty", () => {
  assert.equal(
    diagnosticId(failedMessage()),
    "turn turn-1 / attempts 3 / capacity_wait_ms 61000 / upstream upstream_closed / cause nyxid / admission / route / HTTP 502 / NyxID 12103 / response resp-1 / request req-1",
  );
  assert.match(
    recoveryReason("activity_outcome_uncertain"),
    /may have taken effect/,
  );
  assert.match(recoveryReason("already_retried"), /already retried/);
});

test("context reset, steering, model interruption and billing notices match web copy", () => {
  assert.equal(
    contextResetCopy("user_reset"),
    "You reset the working context.",
  );
  assert.equal(
    contextResetCopy("model_changed"),
    "The agent configuration changed; the assistant context was refreshed.",
  );
  assert.match(contextResetCopy("stopped"), /fresh context with a recap/);
  assert.equal(
    steeringNotice(null, "applied"),
    "Guidance applied to this reply.",
  );
  assert.match(steeringNotice("stop_pending"), /stopping/);
  assert.match(
    storedError("model_interrupted"),
    /Some actions may already have completed/,
  );
  assert.match(
    storedError("capacity_exceeded", 61000),
    /waited 1 minute 1 second/,
  );
  assert.equal(
    billingError("insufficient_credits")?.title,
    "Not enough credits",
  );
  assert.equal(
    billingError("billing_provider_unavailable")?.openBilling,
    false,
  );
});
