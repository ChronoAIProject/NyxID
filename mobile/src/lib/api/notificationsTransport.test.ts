import assert from "node:assert/strict";
import test from "node:test";
import {
  NOTIFICATIONS_PATH,
  createNotificationsApi,
  notificationListPath,
} from "./notificationsTransport";

const C = `nyxa-${"a".repeat(32)}`;
function setup(answers: Record<string, unknown> = {}) {
  const calls: { path: string; options: any }[] = [];
  const api = createNotificationsApi(async (path, options) => {
    calls.push({ path, options });
    const key = `${options?.method ?? "GET"} ${path.split("?")[0]}`;
    if (key in answers) return answers[key];
    return { items: [], next_cursor: null, as_of: "2026-11-10T08:06:00Z" };
  });
  return { api, calls };
}

test("list queries are bounded GETs with the closed kind subset", async () => {
  assert.equal(notificationListPath({}), `${NOTIFICATIONS_PATH}?limit=30`);
  assert.equal(
    notificationListPath({ cursor: "c1", limit: 500, unreadOnly: true, kinds: ["chat_update", "chat_update", "agent_update"] }),
    `${NOTIFICATIONS_PATH}?limit=50&cursor=c1&unread_only=true&kind=chat_update%2Cagent_update`,
  );
  assert.equal(notificationListPath({ limit: 0 }), `${NOTIFICATIONS_PATH}?limit=1`);
  assert.equal(
    notificationListPath({ pendingOnly: true, kinds: ["needs_decision"] }),
    `${NOTIFICATIONS_PATH}?limit=30&pending_only=true&kind=needs_decision`,
    "Needs you never sets unread_only",
  );
  const x = setup({
    [`GET ${NOTIFICATIONS_PATH}`]: {
      items: [
        {
          id: "n1",
          kind: "chat_update",
          source: { type: "channel_chat", id: C, channel_id: "ch", label: "Support" },
          preview: "Hello",
          created_at: "2026-11-10T08:00:00Z",
          updated_at: "2026-11-10T08:05:00Z",
        },
      ],
      next_cursor: "next",
      as_of: "2026-11-10T08:06:00Z",
    },
  });
  const page = await x.api.list({ cursor: null, kinds: ["chat_update"] });
  assert.equal(page.items[0]!.source.label, "Support");
  assert.equal(page.next_cursor, "next");
  assert.equal(x.calls[0]!.options.method, "GET");
  assert.equal(x.calls[0]!.path, `${NOTIFICATIONS_PATH}?limit=30&kind=chat_update`);
});

test("summary and read requests use their exact routes and bodies", async () => {
  const x = setup({ [`GET ${NOTIFICATIONS_PATH}/summary`]: { unread_count: 2, needs_decision_count: 1, by_source: [], by_source_truncated: true } });
  const summary = await x.api.summary();
  assert.equal(summary.unread_count, 2);
  assert.equal(summary.by_source_truncated, true);
  await x.api.markRead({ ids: ["n1", "n2"] });
  await x.api.markRead({ source: { type: "conversation", id: C } });
  await x.api.markRead({ all: true, before: "2026-11-10T08:06:00Z" });
  assert.deepEqual(
    x.calls.slice(1).map((call) => [call.path, call.options.method, call.options.body]),
    [
      [`${NOTIFICATIONS_PATH}/read`, "POST", { ids: ["n1", "n2"] }],
      [`${NOTIFICATIONS_PATH}/read`, "POST", { source: { type: "conversation", id: C } }],
      [`${NOTIFICATIONS_PATH}/read`, "POST", { all: true, before: "2026-11-10T08:06:00Z" }],
    ],
  );
  await assert.rejects(() => x.api.markRead({ ids: [] }));
  assert.equal(x.calls.length, 4, "an invalid scope never reaches the server");
});

test("push preferences read the additive settings slice and write all six switches", async () => {
  let stored: unknown = { access_mode: "nyxmode" };
  const x = setup();
  const api = createNotificationsApi(async (path, options) => {
    x.calls.push({ path, options });
    if (options?.method === "PUT") stored = { access_mode: "nyxmode", ...(options.body as object) };
    return stored;
  });
  const before = await api.pushPreferences();
  assert.equal(before.chat_update, false);
  const after = await api.updatePushPreferences({ ...before, chat_update: true });
  assert.equal(after.chat_update, true);
  assert.deepEqual(x.calls[1]!.options.body, {
    notifications: {
      push: {
        automation_result: false,
        automation_problem: false,
        needs_decision: false,
        agent_update: false,
        chat_update: true,
        group_mention: false,
      },
    },
  });
  assert.equal(x.calls[1]!.path, "/assistant/nyxagent/settings");
});
