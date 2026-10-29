import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema, type AssistantAgent } from "../agents/agentContracts";
import {
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
  assert.equal(automationScheduleSummary(row), "Webhook event");
  assert.equal(
    automationScheduleSummary({
      ...row,
      source: "schedule",
      schedule: {
        kind: "cron",
        expression: "0 8 * * 1-5",
        timezone: "Asia/Singapore",
      },
    }),
    "Weekdays at 8:00 AM (Asia/Singapore)",
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
    }),
    "Every 2 hours",
  );
  assert.equal(
    automationScheduleSummary({
      ...row,
      source: "schedule",
      schedule: { kind: "at", at: run.scheduled_at },
    }),
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
      automationScheduleSummary({
        ...row,
        source: "schedule",
        schedule: { kind: "cron", expression: expression!, timezone: "UTC" },
      }),
      `${description} (UTC)`,
    );
});
