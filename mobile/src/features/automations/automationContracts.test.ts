import assert from "node:assert/strict";
import test from "node:test";
import {
  automationCreateRequestSchema,
  automationPreferencesSchema,
  automationRunSchema,
  automationRunsCursorSchema,
  automationSchema,
  automationSettingsSchema,
  automationUpdateRequestSchema,
  type AutomationCreateRequest,
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
const input: AutomationCreateRequest = {
  label: "Report",
  source: "schedule",
  schedule: { kind: "at", at: "2026-10-01T00:00:00Z" },
  verification: { mode: "schedule" },
  delivery: { type: "notification" },
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
  assert.equal(
    automationUpdateRequestSchema.safeParse({
      verification: { mode: "schedule" },
    }).success,
    false,
  );
  assert.equal(
    automationUpdateRequestSchema.safeParse({ source: "webhook" }).success,
    false,
  );
  assert.equal(
    automationCreateRequestSchema.safeParse({ ...input, source: "webhook" })
      .success,
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

test("owner preferences use documented server defaults and reject unsupported settings writes", () => {
  assert.deepEqual(
    automationSettingsSchema.parse({
      access_mode: "nyxmode",
      skip_destructive_confirmation: true,
    }),
    {
      timezone: null,
      schedule_minimum_minutes: 5,
      trigger_runs_per_hour: 30,
      trigger_runs_per_day: 300,
    },
  );
  const saved = {
    timezone: "Asia/Shanghai",
    schedule_minimum_minutes: 60,
    trigger_runs_per_hour: 12,
    trigger_runs_per_day: 120,
  };
  assert.deepEqual(automationSettingsSchema.parse(saved), saved);
  for (const patch of [
    { timezone: "invalid" },
    { schedule_minimum_minutes: 4 },
    { schedule_minimum_minutes: 1441 },
    { trigger_runs_per_hour: 301 },
    { trigger_runs_per_day: 3001 },
  ])
    assert.equal(
      automationPreferencesSchema.safeParse({ ...saved, ...patch }).success,
      false,
    );
});

test("API uses existing trigger routes, exact verbs and validated request bodies", async () => {
  const calls: { path: string; options: unknown }[] = [];
  const responses: unknown[] = [
    { triggers: [trigger] },
    trigger,
    { trigger, secret: null, delivery_signing_secret: null },
    { trigger, delivery_signing_secret: "one-time-value" },
    run,
    { runs: [run], next_cursor: null },
    { next_runs: ["2026-10-01T00:00:00Z"], description: "Once" },
    {
      label: "Report",
      instruction: "Brief me",
      agent_id: "nyxbot",
      confirmation_policy: "changes",
      thread_policy: null,
    },
    { message: "Deleted" },
  ];
  const api = createAutomationsApi(async (path, options) => {
    calls.push({ path, options });
    return responses.shift();
  });
  await api.list();
  await api.detail(id);
  await api.create(input);
  await api.update(id, { status: "disabled" });
  await api.run(id);
  await api.runs(id, {
    before: "2026-10-01T00:00:00+08:00",
    before_id: otherId,
  });
  await api.preview(input.schedule!);
  await api.setup(id);
  await api.remove(id);
  assert.deepEqual(
    calls.map((call) => call.path),
    [
      "/triggers",
      `/triggers/${id}`,
      "/triggers",
      `/triggers/${id}`,
      `/triggers/${id}/run`,
      `/triggers/${id}/runs?before=2026-10-01T00%3A00%3A00%2B08%3A00&before_id=${otherId}`,
      "/triggers/preview",
      `/triggers/setup/${id}`,
      `/triggers/${id}`,
    ],
  );
  assert.deepEqual(calls[2]!.options, { method: "POST", body: input });
  assert.deepEqual(calls[3]!.options, {
    method: "PATCH",
    body: { status: "disabled" },
  });
  assert.deepEqual(calls[6]!.options, {
    method: "POST",
    body: { schedule: input.schedule },
    signal: undefined,
  });
  assert.deepEqual(calls[8]!.options, { method: "DELETE" });
});

test("API rejects bad outgoing payloads before dispatch and fences wrong trigger identities", async () => {
  let calls = 0;
  const api = createAutomationsApi(async () => {
    calls++;
    return { ...trigger, id: otherId };
  });
  await assert.rejects(api.detail("../elsewhere"));
  await assert.rejects(api.runs(id, { before: "invalid", before_id: id }));
  await assert.rejects(
    api.create({
      ...input,
      delivery: { type: "webhook", url: "http://example.com" },
    }),
  );
  await assert.rejects(api.update(id, { status: "running" as "active" }));
  await assert.rejects(api.updatePreferences({ timezone: "invalid" }));
  assert.equal(calls, 0);
  await assert.rejects(api.detail(id), /different automation/);
  const updates = createAutomationsApi(async () => ({
    trigger: { ...trigger, id: otherId },
    delivery_signing_secret: null,
  }));
  await assert.rejects(
    updates.update(id, { label: "New" }),
    /different automation/,
  );
});

test("channel destinations include only postable server-returned chats and preferences send a partial PUT", async () => {
  const calls: { path: string; options: unknown }[] = [];
  const controller = new AbortController();
  const api = createAutomationsApi(async (path, options) => {
    calls.push({ path, options });
    if (path.endsWith("/channels"))
      return { channel_agents: [{ id: "channel-a" }] };
    if (path.endsWith("/chats"))
      return {
        chats: [
          {
            id: "yes",
            channel_agent_id: "channel-a",
            platform: "telegram",
            bot_label: "Bot",
            title: "Team",
            allow_posts: true,
          },
          {
            id: "no",
            channel_agent_id: "channel-a",
            platform: "telegram",
            bot_label: "Bot",
            allow_posts: false,
          },
          {
            id: "unset",
            channel_agent_id: "channel-a",
            platform: "telegram",
            bot_label: "Bot",
          },
        ],
      };
    return {
      timezone: "Asia/Tokyo",
      schedule_minimum_minutes: 15,
      trigger_runs_per_hour: 20,
      trigger_runs_per_day: 200,
    };
  });
  const chats = await api.chats(controller.signal);
  assert.deepEqual(
    chats.map((chat) => [chat.id, chat.title]),
    [["yes", "Team"]],
  );
  assert.equal(calls[1]!.path, "/assistant/nyxagent/channels/channel-a/chats");
  assert.deepEqual(calls[1]!.options, { signal: controller.signal });
  await api.updatePreferences({ timezone: "Asia/Tokyo" });
  assert.deepEqual(calls.at(-1), {
    path: "/assistant/nyxagent/settings",
    options: { method: "PUT", body: { timezone: "Asia/Tokyo" } },
  });
  const bad = createAutomationsApi(async (path) =>
    path.endsWith("/channels")
      ? { channel_agents: [{ id: "channel-a" }] }
      : {
          chats: [
            {
              id: "chat",
              channel_agent_id: "channel-b",
              platform: "telegram",
              bot_label: "Bot",
              allow_posts: true,
            },
          ],
        },
  );
  await assert.rejects(bad.chats(), /different channel/);
});

test("device targets read the personal conversation endpoint and exclude inactive and bot conversations", async () => {
  const controller = new AbortController();
  const device = {
    id,
    platform: "device",
    platform_conversation_id: "Local worker",
    platform_conversation_type: "device",
    is_active: true,
    allow_agent_initiated: true,
  };
  const api = createAutomationsApi(async (path, options) => {
    assert.equal(path, "/channel-conversations");
    assert.deepEqual(options, { signal: controller.signal });
    return {
      conversations: [
        device,
        { ...device, id: otherId, is_active: false },
        { ...device, id: otherId, platform: "telegram" },
      ],
      total: 3,
    };
  });
  assert.deepEqual(await api.deviceConversations(controller.signal), [device]);
  const invalid = createAutomationsApi(async () => ({
    conversations: [{ ...device, id: "../other" }],
    total: 1,
  }));
  await assert.rejects(invalid.deviceConversations());
});
