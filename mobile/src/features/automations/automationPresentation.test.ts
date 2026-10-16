import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema, type AssistantAgent } from "../agents/agentContracts";
import {
  automationPauseReason,
  automationRunNotice,
  automationTarget,
  ownedAutomationAgents,
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

test("automation agent choices require the current person and exclude org, guest, destroyed and unusable targets", () => {
  const ownNyxBot: AssistantAgent = {
    ...agent,
    id: "nyxbot",
    kind: "nyxbot",
    status: "running",
  };
  const rows: AssistantAgent[] = [
    agent,
    ownNyxBot,
    {
      ...agent,
      id: "shared-person",
      owner_id: "person-b",
      can_maintain: false,
    },
    {
      ...agent,
      id: "other-owner-maintainer",
      owner_id: "person-b",
      can_maintain: true,
    },
    {
      ...agent,
      id: "org-owner",
      owner_kind: "org",
      owner_id: "person-a",
      can_maintain: true,
    },
    { ...agent, id: "destroyed", status: "destroyed" },
    { ...agent, id: "denied-use", can_use: false },
  ];
  assert.deepEqual(
    ownedAutomationAgents(rows, "person-a").map((row) => row.id),
    ["agent-a", "nyxbot"],
  );
  assert.deepEqual(ownedAutomationAgents(rows), []);
  assert.deepEqual(ownedAutomationAgents(rows, ""), []);
  assert.deepEqual(ownedAutomationAgents(rows, "unknown"), []);
});

test("legacy personally listed agents remain available while explicit denied maintenance is excluded", () => {
  const legacy = {
    ...agent,
    owner_id: undefined,
    owner_kind: undefined,
    can_use: undefined,
    can_maintain: undefined,
  };
  assert.deepEqual(ownedAutomationAgents([legacy], "person-a"), [legacy]);
  assert.deepEqual(
    ownedAutomationAgents([{ ...legacy, can_maintain: false }], "person-a"),
    [],
  );
});

test("a run admission notice never claims a queued or started request has completed", () => {
  const queued = automationRunNotice(run);
  assert.match(queued, /queued/i);
  assert.match(queued, /history/i);
  assert.doesNotMatch(queued, /completed|succeeded|finished/i);
  const started = automationRunNotice({ ...run, outcome: "started" });
  assert.match(started, /started/i);
  assert.doesNotMatch(started, /completed|succeeded|finished/i);
  const waiting = automationRunNotice({
    ...run,
    outcome: "waiting",
    reason: "confirmation_required",
  });
  assert.match(waiting, /waiting/i);
  assert.match(waiting, /review|confirmation/i);
  for (const outcome of ["completed", "failed", "skipped"] as const)
    assert.match(automationRunNotice({ ...run, outcome }), new RegExp(outcome));
});

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
