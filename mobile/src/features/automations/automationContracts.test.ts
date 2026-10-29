import assert from "node:assert/strict";
import test from "node:test";
import {
  automationRunSchema,
  automationRunsCursorSchema,
  automationSchema,
  automationSettingsSchema,
} from "./automationContracts";
import { createAutomationsApi } from "../../lib/api/automationsTransport";

const id = "12345678-1234-4123-8123-123456789abc";
const otherId = "12345678-1234-4123-8123-123456789abd";
const trigger = {
  id,
  user_id: "owner",
  label: "Report",
  user_service_id: null,
  status: "active",
  source: "schedule",
  schedule: { kind: "at", at: "2026-10-01T00:00:00Z" },
  verification: { mode: "schedule" },
  delivery: { type: "notification" },
  inbound_url: null,
  created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z",
};
const run = {
  id,
  scheduled_at: "2026-10-01T00:00:00Z",
  started_at: null,
  finished_at: null,
  outcome: "pending",
  reason: null,
  thread_id: null,
  duration_ms: null,
};
test("parsers reject unknown status, deliveries, malformed timestamps and unbounded references", () => {
  assert.equal(automationSchema.safeParse(trigger).success, true);
  for (const change of [
    { status: "paused" },
    { delivery: { type: "email" } },
    { id: "invalid" },
    { next_run_at: "tomorrow" },
    { source: "scheduler" },
    { delivery: { type: "agent", conversation_id: "x".repeat(257) } },
  ])
    assert.equal(
      automationSchema.safeParse({ ...trigger, ...change }).success,
      false,
    );
  assert.equal(
    automationRunsCursorSchema.safeParse({
      before: "2026-10-01T00:00:00Z",
      before_id: "../secret",
    }).success,
    false,
  );
});

test("notification runs normalize the scheduler's empty thread sentinel in history and detail", async () => {
  const completed = {
    ...run,
    outcome: "completed",
    started_at: "2026-10-01T00:00:00Z",
    finished_at: "2026-10-01T00:00:01Z",
    duration_ms: 1000,
    thread_id: "",
  };
  const api = createAutomationsApi(async (path) =>
    path.endsWith("/runs")
      ? { runs: [completed], next_cursor: null }
      : { ...trigger, last_run: completed },
  );
  const history = await api.runs(id);
  const detail = await api.detail(id);
  assert.equal(history.runs[0]!.thread_id, null);
  assert.equal(detail.last_run!.thread_id, null);
  // The screen renders Open thread only when thread_id is truthy.
  assert.equal(
    history.runs.some((item) => !!item.thread_id),
    false,
  );
  assert.equal(
    automationRunSchema.parse({ ...completed, thread_id: "nyxa-real-thread" })
      .thread_id,
    "nyxa-real-thread",
  );
  for (const thread_id of ["has whitespace", "x".repeat(257)])
    assert.equal(
      automationRunSchema.safeParse({ ...completed, thread_id }).success,
      false,
    );
});

test("display settings read only the saved timezone and fall back when absent", () => {
  assert.deepEqual(automationSettingsSchema.parse({ access_mode: "nyxmode" }), {
    timezone: null,
  });
  assert.deepEqual(
    automationSettingsSchema.parse({
      timezone: "Asia/Shanghai",
      trigger_runs_per_day: 300,
    }),
    { timezone: "Asia/Shanghai" },
  );
  assert.equal(
    automationSettingsSchema.safeParse({ timezone: "invalid" }).success,
    false,
  );
});

test("every native automation API operation is GET-only, with exact cursor paths and cancellation", async () => {
  const controller = new AbortController();
  const calls: { path: string; options: any }[] = [];
  const responses = [
    { triggers: [trigger] },
    { triggers: [] },
    trigger,
    {
      runs: [run],
      next_cursor: { before: run.scheduled_at, before_id: run.id },
    },
    { runs: [], next_cursor: null },
    { timezone: "UTC" },
  ];
  const api = createAutomationsApi(async (path, options) => {
    calls.push({ path, options });
    return responses.shift();
  });
  assert.deepEqual(Object.keys(api).sort(), [
    "detail",
    "list",
    "runs",
    "settings",
  ]);
  await api.list(controller.signal);
  await api.list(controller.signal, otherId);
  await api.detail(id, controller.signal);
  const page = await api.runs(id, undefined, controller.signal);
  await api.runs(id, page.next_cursor!, controller.signal);
  await api.settings(controller.signal);
  assert.deepEqual(
    calls.map((call) => call.path),
    [
      "/triggers",
      `/triggers?org_id=${otherId}`,
      `/triggers/${id}`,
      `/triggers/${id}/runs`,
      `/triggers/${id}/runs?before=2026-10-01T00%3A00%3A00Z&before_id=${id}`,
      "/assistant/nyxagent/settings",
    ],
  );
  for (const call of calls)
    assert.deepEqual(call.options, {
      method: "GET",
      signal: controller.signal,
    });
});

test("API rejects invalid identifiers and cursors before dispatch and fences wrong detail identity", async () => {
  let calls = 0;
  const api = createAutomationsApi(async () => {
    calls++;
    return { ...trigger, id: otherId };
  });
  await assert.rejects(api.detail("../elsewhere"));
  await assert.rejects(api.list(undefined, "invalid"));
  await assert.rejects(api.runs(id, { before: "invalid", before_id: id }));
  assert.equal(calls, 0);
  await assert.rejects(api.detail(id), /different automation/);
});
