import assert from "node:assert/strict";
import test from "node:test";
import { automationSchema, type Automation } from "./automationContracts";
import {
  automationDefaults,
  automationFormSchema,
  automationRequest,
  automationUpdateRequest,
  formSchedule,
} from "./automationForm";

const row: Automation = automationSchema.parse({
  id: "12345678-1234-4123-8123-123456789abc",
  user_id: "owner",
  label: "Morning briefing",
  user_service_id: null,
  status: "active",
  source: "schedule",
  schedule: {
    kind: "every",
    amount: 2,
    unit: "hours",
    anchor: "2026-11-01T06:30:23Z",
    start: "2026-11-01T06:30:34Z",
    end: "2026-11-02T06:30:56Z",
    max_runs: 17,
    grace_seconds: 60,
  },
  overlap: "queue",
  verification: { mode: "schedule" },
  delivery: {
    type: "assistant",
    agent_id: "agent",
    instruction: "Brief me",
    thread_policy: "dedicated",
    confirmation_policy: "destructive",
    deliver_to: { type: "chat", chat_id: "chat" },
  },
  inbound_url: null,
  created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z",
});

test("all schedule kinds serialize UTC instants, limits and overlap using Web contract", () => {
  const defaults = {
    ...automationDefaults("Asia/Shanghai"),
    label: "Report",
    delivery_type: "notification" as const,
    timezone: "Asia/Shanghai",
    start: "2026-10-01T08:00",
    end: "2026-10-03T08:00",
    max_runs: "9",
    grace_seconds: "45",
    overlap: "queue" as const,
  };
  assert.deepEqual(
    formSchedule({ ...defaults, kind: "cron", expression: "0 8 * * 1-5" }),
    {
      kind: "cron",
      expression: "0 8 * * 1-5",
      timezone: "Asia/Shanghai",
      start: "2026-10-01T00:00:00.000Z",
      end: "2026-10-03T00:00:00.000Z",
      max_runs: 9,
      grace_seconds: 45,
    },
  );
  const interval = automationRequest({
    ...defaults,
    kind: "every",
    amount: 3,
    unit: "days",
    anchor: "2026-10-01T09:00",
  });
  assert.equal(interval.schedule?.kind, "every");
  assert.equal(
    interval.schedule?.kind === "every" && interval.schedule.anchor,
    "2026-10-01T01:00:00.000Z",
  );
  assert.equal(interval.overlap, "queue");
  assert.deepEqual(interval.verification, { mode: "schedule" });
  assert.deepEqual(
    formSchedule({ ...defaults, kind: "at", at: "2026-10-02T10:00" }),
    {
      kind: "at",
      at: "2026-10-02T02:00:00.000Z",
      start: "2026-10-01T00:00:00.000Z",
      end: "2026-10-03T00:00:00.000Z",
      max_runs: 9,
      grace_seconds: 45,
    },
  );
});

test("creation preserves all delivery types, assistant policies and webhook verification", () => {
  const values = automationDefaults("America/New_York", row);
  assert.deepEqual(automationRequest(values).delivery, row.delivery);
  assert.deepEqual(
    automationRequest({ ...values, delivery_type: "notification" }).delivery,
    { type: "notification" },
  );
  assert.deepEqual(
    automationRequest({
      ...values,
      delivery_type: "agent",
      conversation_id: "device-conversation",
    }).delivery,
    { type: "agent", conversation_id: "device-conversation" },
  );
  assert.deepEqual(
    automationRequest({
      ...values,
      source: "webhook",
      delivery_type: "webhook",
      webhook_url: "https://example.com/hook",
      verification_mode: "hmac",
      signature_header: "X-Signature",
    }),
    {
      label: row.label,
      source: "webhook",
      overlap: "queue",
      verification: { mode: "hmac_sha256", header_name: "X-Signature" },
      delivery: { type: "webhook", url: "https://example.com/hook" },
    },
  );
  assert.equal(
    automationRequest({ ...values, thread_policy: "default" }).delivery.type ===
      "assistant",
    true,
  );
});

test("editing a label or delivery keeps exact schedule timestamps and immutable ingress fields untouched", () => {
  const values = automationDefaults("America/New_York", row);
  assert.equal(values.anchor, "2026-11-01T01:30");
  assert.deepEqual(automationUpdateRequest(values, row), {});
  assert.deepEqual(
    automationUpdateRequest({ ...values, label: "New label" }, row),
    { label: "New label" },
  );
  assert.deepEqual(
    automationUpdateRequest({ ...values, instruction: "New instruction" }, row),
    { delivery: { ...row.delivery, instruction: "New instruction" } },
  );
  const patch = automationUpdateRequest({ ...values, amount: 4 }, row);
  assert.deepEqual(patch, { schedule: { ...row.schedule, amount: 4 } });
  assert.throws(
    () => automationUpdateRequest({ ...values, source: "webhook" }, row),
    /source cannot be changed/,
  );
});

test("clearing options uses null and a cron timezone consistently controls its calendar fields", () => {
  const cron: Automation = {
    ...row,
    schedule: {
      kind: "cron",
      expression: "0 8 * * 1-5",
      timezone: "Asia/Tokyo",
      start: "2026-10-01T00:00:34Z",
      max_runs: 7,
      grace_seconds: 90,
    },
  };
  const values = automationDefaults("America/New_York", cron);
  assert.equal(values.timezone, "Asia/Tokyo");
  assert.equal(values.start, "2026-10-01T09:00");
  assert.deepEqual(
    automationUpdateRequest(
      { ...values, max_runs: "", grace_seconds: "", start: "" },
      cron,
    ),
    {
      schedule: {
        kind: "cron",
        expression: "0 8 * * 1-5",
        timezone: "Asia/Tokyo",
        start: null,
        end: null,
        max_runs: null,
        grace_seconds: null,
      },
    },
  );
});

test("validation enforces byte limits, schedule bounds, public HTTPS and valid HMAC header names", () => {
  const values = automationDefaults("UTC", row);
  for (const patch of [
    { label: "" },
    { label: "字".repeat(43) },
    { instruction: "字".repeat(2731) },
    { max_runs: "1.5" },
    { grace_seconds: "86401" },
    { amount: 0 },
    { end: "2026-10-01T00:00" },
    { delivery_type: "webhook", webhook_url: "http://example.com" },
    {
      delivery_type: "webhook",
      webhook_url: "https://user:password@example.com",
    },
    {
      source: "webhook",
      verification_mode: "hmac",
      signature_header: "Invalid Header",
    },
  ])
    assert.equal(
      automationFormSchema.safeParse({ ...values, ...patch }).success,
      false,
    );
  assert.equal(
    automationFormSchema.safeParse({
      ...values,
      kind: "cron",
      expression: "0 0 8 * * *",
    }).success,
    false,
  );
  assert.equal(
    automationFormSchema.safeParse({
      ...values,
      kind: "cron",
      timezone: "invalid",
    }).success,
    false,
  );
});
