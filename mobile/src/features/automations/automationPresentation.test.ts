import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema, type AssistantAgent } from "../agents/agentContracts";
import {
  automationListDetail,
  automationRunDetail,
  automationStatusLabel,
  automationPauseReason,
  automationTarget,
  automationScheduleSummary,
  uniqueAutomationRuns,
} from "./automationPresentation";
import {
  automationSchema,
  type AutomationRun,
  type AutomationRunsPage,
} from "./automationContracts";

const agent: AssistantAgent = agentSchema.parse({
  id: "agent-a",
  kind: "specialist",
  name: "researcher",
  display_name: "Research helper",
  description: "Research",
  status: "idle",
  owner_id: "person-a",
  owner_kind: "person",
  can_use: true,
  can_maintain: true,
});
const run: AutomationRun = {
  id: "12345678-1234-4123-8123-123456789abc",
  scheduled_at: "2026-10-15T00:00:00Z",
  started_at: null,
  finished_at: null,
  outcome: "pending",
  reason: null,
  thread_id: null,
  duration_ms: null,
};

test("history cursor overlap deduplicates by run identity, preserves newest results and keeps simultaneous distinct runs", () => {
  const older = {
    ...run,
    id: "12345678-1234-4123-8123-123456789abd",
    outcome: "completed" as const,
  };
  const newest = { ...run, outcome: "completed" as const, duration_ms: 450 };
  const pages: AutomationRunsPage[] = [
    {
      runs: [newest],
      next_cursor: { before: run.scheduled_at, before_id: run.id },
    },
    { runs: [run, older], next_cursor: null },
    { runs: [older], next_cursor: null },
  ];
  const snapshot = JSON.stringify(pages);
  assert.deepEqual(uniqueAutomationRuns(pages), [newest, older]);
  assert.equal(uniqueAutomationRuns(pages)[0]!.outcome, "completed");
  assert.equal(JSON.stringify(pages), snapshot);
  assert.deepEqual(uniqueAutomationRuns([]), []);
});

test("destination labels use returned agent names and remain useful after an agent disappears", () => {
  const row = automationSchema.parse({
    id: run.id,
    user_id: "person-a",
    label: "Report",
    user_service_id: null,
    status: "active",
    verification: { mode: "token", location: "bearer" },
    delivery: {
      type: "assistant",
      agent_id: agent.id,
      instruction: "Brief me",
      deliver_to: { type: "thread" },
    },
    inbound_url: "https://example.com/triggers/inbound",
    created_at: run.scheduled_at,
    updated_at: run.scheduled_at,
  });
  assert.equal(automationTarget(row, [agent]), "Research helper");
  assert.equal(
    automationTarget(row, [{ ...agent, display_name: null }]),
    "researcher",
  );
  assert.equal(automationTarget(row), agent.id);
  assert.equal(
    automationTarget({ ...row, delivery: { type: "notification" } }),
    "Reminder",
  );
  assert.equal(
    automationTarget({
      ...row,
      delivery: { type: "agent", conversation_id: "device" },
    }),
    "Device agent",
  );
  assert.equal(
    automationTarget({
      ...row,
      delivery: { type: "webhook", url: "https://example.com/private" },
    }),
    "HTTPS webhook",
  );
  assert.match(automationPauseReason("target_unavailable")!, /Agent access/);
  assert.equal(
    automationPauseReason("future_pause_reason"),
    "future pause reason",
  );
  assert.equal(automationPauseReason(null), undefined);
});

test("schedule summaries distinguish webhook, cron, interval and one-time tasks", () => {
  const row = automationSchema.parse({
    id: run.id,
    user_id: "owner",
    label: "Task",
    user_service_id: null,
    status: "active",
    verification: { mode: "schedule" },
    delivery: { type: "notification" },
    inbound_url: null,
    created_at: run.scheduled_at,
    updated_at: run.scheduled_at,
  });
  assert.equal(automationScheduleSummary(row, "UTC"), "Webhook event");
  assert.equal(
    automationScheduleSummary(
      {
        ...row,
        source: "schedule",
        schedule: {
          kind: "cron",
          expression: "0 8 * * 1-5",
          timezone: "Asia/Singapore",
        },
      },
      "Asia/Singapore",
    ),
    "Weekdays at 8:00 AM",
  );
  assert.equal(
    automationScheduleSummary({
      ...row,
      source: "schedule",
      schedule: {
        kind: "every",
        amount: 2,
        unit: "hours",
        anchor: run.scheduled_at,
      },
    }, "UTC"),
    "Every 2 hours",
  );
  assert.equal(
    automationScheduleSummary({
      ...row,
      source: "schedule",
      schedule: { kind: "at", at: run.scheduled_at },
    }, "UTC"),
    "One time",
  );
  for (const [expression, description] of [
    ["15 0 * * *", "Every day at 12:15 AM"],
    ["0 12 * * 5", "Fridays at 12:00 PM"],
    ["30 21 * * 0,6", "Weekends at 9:30 PM"],
    ["5 8 15 * *", "Monthly on day 15 at 8:05 AM"],
    ["*/15 * * * *", "Custom calendar schedule"],
    ["0 8 1 * 1-5", "Custom calendar schedule"],
    ["0 24 * * *", "Custom calendar schedule"],
  ])
    assert.equal(
      automationScheduleSummary(
        {
          ...row,
          source: "schedule",
          schedule: { kind: "cron", expression: expression!, timezone: "UTC" },
        },
        "UTC",
      ),
      description,
    );
});

test("schedules read in the phone's local time, moving the days when the time crosses midnight", () => {
  const base = automationSchema.parse({
    id: "12345678-1234-4123-8123-123456789abc",
    user_id: "owner",
    label: "Task",
    user_service_id: null,
    status: "active",
    source: "schedule",
    schedule: { kind: "cron", expression: "0 7 * * *", timezone: "America/Los_Angeles" },
    verification: { mode: "schedule" },
    delivery: { type: "notification" },
    inbound_url: null,
    created_at: "2026-10-15T00:00:00Z",
    updated_at: "2026-10-15T00:00:00Z",
  });
  const local = (expression: string, nextRun: string | null, zone = "Asia/Singapore") =>
    automationScheduleSummary(
      {
        ...base,
        next_run_at: nextRun,
        schedule: { kind: "cron", expression, timezone: "America/Los_Angeles" },
      },
      zone,
    );
  // 7:00 AM in Los Angeles (PST) is 11:00 PM the same day in Singapore.
  assert.equal(local("0 7 * * *", "2026-11-04T15:00:00Z"), "Every day at 11:00 PM");
  // 8:00 AM weekdays in Los Angeles is midnight Tuesday–Saturday in Singapore.
  assert.equal(local("0 8 * * 1-5", "2026-11-04T16:00:00Z"), "Tue–Sat at 12:00 AM");
  // 5:00 PM Fridays in Los Angeles is 9:00 AM Saturdays in Singapore.
  assert.equal(local("0 17 * * 5", "2026-11-07T01:00:00Z"), "Saturdays at 9:00 AM");
  // The same zone reads unchanged, with no zone label.
  assert.equal(
    local("0 17 * * 5", "2026-11-07T01:00:00Z", "America/Los_Angeles"),
    "Fridays at 5:00 PM",
  );
  // Daylight saving is taken from the actual next run (PDT in October).
  assert.equal(local("0 7 * * *", "2026-10-20T14:00:00Z"), "Every day at 10:00 PM");
  // A day of the month cannot move across midnight; it names its own zone.
  assert.equal(
    local("5 20 15 * *", "2026-11-16T04:05:00Z"),
    "Monthly on day 15 at 8:05 PM America/Los_Angeles time",
  );
  assert.equal(local("5 7 15 * *", "2026-11-15T15:05:00Z"), "Monthly on day 15 at 11:05 PM");
});

test("list and run rows describe status, timing and outcome as label/value text", () => {
  assert.equal(automationStatusLabel("disabled"), "Paused");
  assert.equal(automationStatusLabel("active"), "Active");
  assert.equal(automationStatusLabel("waiting"), "Waiting");
  const scheduled = automationSchema.parse({
    id: "12345678-1234-4123-8123-123456789abc",
    user_id: "owner",
    label: "Daily",
    user_service_id: null,
    status: "disabled",
    source: "schedule",
    schedule: { kind: "cron", expression: "0 8 * * *", timezone: "UTC" },
    verification: { mode: "schedule" },
    delivery: { type: "notification" },
    pause_reason: "owner_unavailable",
    inbound_url: null,
    created_at: "2026-10-15T00:00:00Z",
    updated_at: "2026-10-15T00:00:00Z",
  });
  assert.equal(
    automationListDetail(scheduled, "Reminder", "UTC"),
    "Reminder · Scheduled\nEvery day at 8:00 AM\nPaused · no new scheduled runs\nThe owner account is unavailable.",
  );
  assert.equal(
    automationRunDetail({
      ...run,
      reason: "confirmation_required",
      missed: { count: 3, from: run.scheduled_at, through: run.scheduled_at },
      duration_ms: 1500,
    }),
    "confirmation required · Missed 3 occurrences · 1.5 seconds",
  );
  assert.equal(
    automationRunDetail({
      ...run,
      reason: null,
      missed: null,
      duration_ms: null,
    }),
    "",
  );
});
