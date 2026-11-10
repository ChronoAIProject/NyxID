import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import type {
  useNotificationIndicator,
  useNotificationsLive,
  useVisibleThreadRead,
} from "./useNotifications";

// The hook module touches React Native at import time, so every test loads it
// through the harness and reads its constants from that loaded copy.
const NOTIFICATIONS = ["workspace", "notifications"];
const VISIBLE_THREAD_READ_DELAY_MS = 1_500;
const C = `nyxa-${"a".repeat(32)}`;
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function setup(options: {
  features?: string[];
  summary?: unknown;
  dataUpdatedAt?: number;
  focused?: boolean;
  active?: boolean;
  live?: boolean;
} = {}) {
  const state = {
    features: options.features ?? ["assistant:notifications", "assistant:nyxagent-engine"],
    summary: options.summary as unknown,
    dataUpdatedAt: options.dataUpdatedAt ?? 1,
    focused: options.focused ?? true,
    /** Screen focused and app in front; toggled by tests for pauses. */
    active: options.active ?? true,
    live: options.live ?? false,
    epoch: 7,
    source: { type: "conversation", id: C } as { type: "conversation" | "group"; id: string } | undefined,
  };
  const reads: unknown[] = [];
  const feeds: { queryKey: unknown[]; enabled?: boolean; queryFn: (context: { pageParam?: string; signal?: AbortSignal }) => unknown }[] = [];
  const lists: unknown[] = [];
  const invalidations: unknown[] = [];
  const queries: { key: unknown; enabled: unknown; refetchInterval: unknown }[] = [];
  let onLiveEvent: ((event: unknown) => void) | undefined;
  const h = hookHarness<unknown>({
    "react-native": {
      AppState: { currentState: "active", addEventListener: () => ({ remove() {} }) },
    },
    "@react-navigation/native": { useIsFocused: () => state.focused },
    "@tanstack/react-query": {
      useQuery: (options: { queryKey: unknown[]; enabled: unknown; refetchInterval: unknown }) => {
        queries.push({ key: options.queryKey, enabled: options.enabled, refetchInterval: options.refetchInterval });
        if (options.queryKey[0] === "account")
          return { data: { capabilities: { enabled_features: state.features } } };
        return { data: state.summary, dataUpdatedAt: state.dataUpdatedAt };
      },
      useInfiniteQuery: (options: { queryKey: unknown[]; enabled?: boolean; queryFn: (context: { pageParam?: string; signal?: AbortSignal }) => unknown }) => {
        feeds.push(options);
        return { data: undefined };
      },
      useMutation: () => ({ mutate: (value: unknown) => reads.push(value) }),
      useQueryClient: () => ({
        invalidateQueries: (value: unknown) => invalidations.push(value),
        setQueryData: () => {},
      }),
    },
    "../../hooks/useNetworkStatus": { useNetworkStatus: () => ({ isConnected: true }) },
    "../../lib/api/assistantLiveTransport": { livePollInterval },
    "../../lib/api/http": { getCurrentUserProfileRequest: async () => ({}) },
    "../../lib/api/notificationsApi": {
      notificationsApi: { list: async (params: unknown) => { lists.push(params); return { items: [], next_cursor: null, as_of: "x" }; } },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => state.epoch },
    "../groups/useGroupsLive": {
      useGroupsLive: (_epoch: number, _active: boolean, _available: unknown, onEvent?: (event: unknown) => void) => {
        onLiveEvent = onEvent;
        return state.live;
      },
    },
  });
  const module = h.load(fileURLToPath(new URL("./useNotifications.ts", import.meta.url))) as {
    useVisibleThreadRead: typeof useVisibleThreadRead;
    useNotificationIndicator: typeof useNotificationIndicator;
    useNotificationsLive: typeof useNotificationsLive;
    useNotificationFeed: (filter: string, enabled: boolean, active: boolean, live?: boolean) => unknown;
    useNotificationsEnabled: (active?: boolean) => boolean;
    NOTIFICATIONS: readonly string[];
    VISIBLE_THREAD_READ_DELAY_MS: number;
  };
  assert.deepEqual([...module.NOTIFICATIONS], NOTIFICATIONS);
  assert.equal(module.VISIBLE_THREAD_READ_DELAY_MS, VISIBLE_THREAD_READ_DELAY_MS);
  return { h, module, state, reads, feeds, lists, invalidations, queries, liveEvent: (event: unknown) => onLiveEvent?.(event) };
}

test("a visible thread is read once it stays in front, re-arms on new exact unread and never fires while hidden", async () => {
  const x = setup({
    summary: { unread_count: 3, needs_decision_count: 0, by_source_truncated: false,
      by_source: [{ source_type: "channel_chat", id: C, conversation_id: C, unread: 2 }] },
  });
  x.h.mount(() => x.module.useVisibleThreadRead({ type: "conversation", id: C }, true, true));
  assert.deepEqual(x.reads, [], "nothing before the debounce");
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.deepEqual(JSON.parse(JSON.stringify(x.reads)), [{ source: { type: "conversation", id: C } }]);
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(x.reads.length, 1, "the same summary is not read twice");
  x.state.dataUpdatedAt = 2;
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(x.reads.length, 2, "fresh unread for this thread reads again");
  x.h.unmount();
  const hidden = setup({
    focused: false,
    summary: { unread_count: 3, needs_decision_count: 0, by_source_truncated: false,
      by_source: [{ source_type: "conversation", id: C, conversation_id: null, unread: 1 }] },
  });
  hidden.h.mount(() => hidden.module.useVisibleThreadRead({ type: "conversation", id: C }, true, false));
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.deepEqual(hidden.reads, [], "an inactive thread never reads");
  hidden.h.unmount();
});

test("without an exact entry a thread reads only when the bounded summary is truncated, and unmount cancels the pending read", async () => {
  const exact = setup({
    summary: { unread_count: 4, needs_decision_count: 1, by_source_truncated: false,
      by_source: [{ source_type: "trigger", id: "t", conversation_id: `nyxa-${"b".repeat(32)}`, unread: 4 }] },
  });
  exact.h.mount(() => exact.module.useVisibleThreadRead({ type: "conversation", id: C }, true, true));
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.deepEqual(exact.reads, [], "other threads' rows do not trigger a read here");
  exact.h.unmount();
  const truncated = setup({
    summary: { unread_count: 4, needs_decision_count: 0, by_source_truncated: true, by_source: [] },
  });
  truncated.h.mount(() => truncated.module.useVisibleThreadRead({ type: "group", id: `nyxg-${"c".repeat(32)}` }, true, true));
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.deepEqual(JSON.parse(JSON.stringify(truncated.reads)), [{ source: { type: "group", id: `nyxg-${"c".repeat(32)}` } }]);
  truncated.state.dataUpdatedAt = 9;
  truncated.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(truncated.reads.length, 1, "the fallback reads once per open");
  truncated.h.unmount();
  const cancelled = setup({
    summary: { unread_count: 1, needs_decision_count: 0, by_source_truncated: false,
      by_source: [{ source_type: "conversation", id: C, conversation_id: null, unread: 1 }] },
  });
  cancelled.h.mount(() => cancelled.module.useVisibleThreadRead({ type: "conversation", id: C }, true, true));
  cancelled.h.unmount();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.deepEqual(cancelled.reads, [], "leaving the thread cancels the pending read");
});

test("the indicator follows the profile flag, caps its badge and polls the summary only when enabled and in front", () => {
  const on = setup({ summary: { unread_count: 12, needs_decision_count: 1, by_source: [], by_source_truncated: false } });
  const value = on.h.mount(() => on.module.useNotificationIndicator(true)) as ReturnType<typeof useNotificationIndicator>;
  assert.deepEqual({ enabled: value.enabled, unread: value.unread, needsDecision: value.needsDecision, badge: value.badge },
    { enabled: true, unread: 12, needsDecision: 1, badge: "9+" });
  const summary = on.queries.find((query) => JSON.stringify(query.key) === JSON.stringify([...NOTIFICATIONS, "summary", 7]))!;
  assert.equal(summary.enabled, true);
  assert.equal(summary.refetchInterval, 30_000, "a 30 second backstop while the stream is down");
  on.h.unmount();
  const off = setup({ features: [], summary: { unread_count: 5, needs_decision_count: 0, by_source: [], by_source_truncated: false } });
  const hidden = off.h.mount(() => off.module.useNotificationIndicator(true)) as ReturnType<typeof useNotificationIndicator>;
  assert.equal(hidden.enabled, false);
  const offSummary = off.queries.find((query) => JSON.stringify(query.key) === JSON.stringify([...NOTIFICATIONS, "summary", 7]))!;
  assert.equal(offSummary.enabled, false, "no summary request while the flag is off");
  off.h.unmount();
  const background = setup({ live: true, summary: { unread_count: 1, needs_decision_count: 0, by_source: [], by_source_truncated: false } });
  background.h.mount(() => background.module.useNotificationIndicator(false));
  const bg = background.queries.find((query) => JSON.stringify(query.key) === JSON.stringify([...NOTIFICATIONS, "summary", 7]))!;
  assert.equal(bg.enabled, false);
  assert.equal(bg.refetchInterval, false);
  background.h.unmount();
});

test("with the notifications flag on but the NyxAgent engine off nothing is enabled and no feed or summary is requested", () => {
  const x = setup({ features: ["assistant:notifications"], summary: { unread_count: 9, needs_decision_count: 2, by_source: [], by_source_truncated: false } });
  const value = x.h.mount(() => ({
    indicator: x.module.useNotificationIndicator(true),
    feed: x.module.useNotificationFeed("all", x.module.useNotificationsEnabled(true), true),
  })) as { indicator: ReturnType<typeof useNotificationIndicator> };
  assert.equal(value.indicator.enabled, false);
  assert.deepEqual({ unread: value.indicator.unread, badge: value.indicator.badge }, { unread: 9, badge: "9" }, "the harness summary is still reported; production never fetches it");
  const summary = x.queries.find((query) => JSON.stringify(query.key) === JSON.stringify([...NOTIFICATIONS, "summary", 7]))!;
  assert.equal(summary.enabled, false, "no summary request without the engine");
  assert.equal(summary.refetchInterval, false);
  assert.equal((x.feeds.at(-1) as { enabled?: boolean }).enabled, false, "no feed request without the engine");
  assert.deepEqual(x.lists, []);
  x.h.unmount();
  const engineOnly = setup({ features: ["assistant:nyxagent-engine"] });
  const hidden = engineOnly.h.mount(() => engineOnly.module.useNotificationIndicator(true)) as ReturnType<typeof useNotificationIndicator>;
  assert.equal(hidden.enabled, false, "the engine alone does not enable notifications");
  engineOnly.h.unmount();
});

test("the indicator derives sidebar markers once per summary", () => {
  const x = setup({ summary: { unread_count: 3, needs_decision_count: 0, by_source_truncated: false,
    by_source: [{ source_type: "channel_chat", id: C, conversation_id: C, unread: 2 }, { source_type: "group", id: `nyxg-${"d".repeat(32)}`, conversation_id: null, unread: 1 }] } });
  const value = x.h.mount(() => x.module.useNotificationIndicator(true)) as ReturnType<typeof useNotificationIndicator>;
  assert.deepEqual(JSON.parse(JSON.stringify(value.markers)), { conversations: { [C]: 2 }, groups: { [`nyxg-${"d".repeat(32)}`]: 1 } });
  x.h.unmount();
});

test("notification and resync frames invalidate every notification query once per burst; other frames do not", async () => {
  const x = setup();
  x.h.mount(() => x.module.useNotificationsLive(true));
  x.liveEvent({ type: "conversation", id: C, group_id: null, turn_id: null, messages: 1 });
  await wait(200);
  assert.deepEqual(x.invalidations, []);
  x.liveEvent({ type: "notification", id: "n1" });
  x.liveEvent({ type: "notification", id: "n2" });
  x.liveEvent({ type: "resync" });
  await wait(250);
  assert.deepEqual(JSON.parse(JSON.stringify(x.invalidations)), [{ queryKey: NOTIFICATIONS }]);
  x.h.unmount();
  x.liveEvent({ type: "notification", id: "n3" });
  await wait(250);
  assert.equal(x.invalidations.length, 2, "events after unmount still coalesce through the shared stream");
});

test("Needs you asks the server for waiting decisions only; other filters keep history", async () => {
  const x = setup();
  x.h.mount(() => x.module.useNotificationFeed("needs_you", true, true));
  await x.feeds.at(-1)!.queryFn({ pageParam: "cur" });
  x.h.unmount();
  const all = setup();
  all.h.mount(() => all.module.useNotificationFeed("automations", true, true));
  await all.feeds.at(-1)!.queryFn({});
  all.h.unmount();
  assert.deepEqual(JSON.parse(JSON.stringify([...x.lists, ...all.lists])), [
    { cursor: "cur", kinds: ["needs_decision"], pendingOnly: true },
    { kinds: ["automation_result", "automation_problem"], pendingOnly: false },
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(x.feeds.at(-1)!.queryKey)), [...NOTIFICATIONS, "feed", 7, "needs_you"]);
});

const exactSummary = (unread: number) => ({
  unread_count: unread, needs_decision_count: 0, by_source_truncated: false,
  by_source: unread ? [{ source_type: "channel_chat", id: C, conversation_id: C, unread }] : [],
});
const truncatedSummary = { unread_count: 4, needs_decision_count: 0, by_source_truncated: true, by_source: [] };
/** Mounts the hook on the harness state so tests can pause, blur and switch threads. */
const mountVisible = (x: ReturnType<typeof setup>) =>
  x.h.mount(() => x.module.useVisibleThreadRead(x.state.source, true, x.state.active));
const refresh = (x: ReturnType<typeof setup>, summary: unknown) => {
  x.state.summary = summary;
  x.state.dataUpdatedAt += 1;
  x.h.render();
};

test("summary refreshes before the deadline neither restart nor skip it, and after it each refresh is judged at once", async () => {
  const x = setup({ summary: exactSummary(2) });
  mountVisible(x);
  for (let i = 0; i < 3; i++) {
    await wait(400);
    refresh(x, exactSummary(2));
  }
  assert.deepEqual(x.reads, [], "still inside the debounce");
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150 - 1200);
  assert.equal(x.reads.length, 1, "a busy feed cannot postpone the open thread's read");
  refresh(x, exactSummary(5));
  assert.equal(x.reads.length, 2, "once due, a changed count reads without waiting again");
  x.h.unmount();
});

test("an unread count the server leaves unchanged is asked about three times, then only a change or a clear re-arms", async () => {
  const x = setup({ summary: exactSummary(2) });
  mountVisible(x);
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(x.reads.length, 1);
  refresh(x, exactSummary(2));
  refresh(x, exactSummary(2));
  assert.equal(x.reads.length, 3, "two more tries for the same count");
  refresh(x, exactSummary(2));
  refresh(x, exactSummary(2));
  assert.equal(x.reads.length, 3, "then the thread stops asking");
  refresh(x, exactSummary(3));
  assert.equal(x.reads.length, 4, "a changed count is new work");
  refresh(x, exactSummary(3));
  refresh(x, exactSummary(3));
  refresh(x, exactSummary(3));
  assert.equal(x.reads.length, 6, "the budget restarted with the new count");
  refresh(x, exactSummary(0));
  assert.equal(x.reads.length, 6, "nothing to read once cleared");
  refresh(x, exactSummary(3));
  assert.equal(x.reads.length, 7, "after a clear the same count is fresh again");
  x.h.unmount();
});

test("a background pause keeps what this open already read and re-arms the debounce; a navigation blur and return is a new open", async () => {
  const x = setup({ summary: truncatedSummary });
  mountVisible(x);
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(x.reads.length, 1, "truncated summaries read once per open");
  x.state.active = false;
  x.h.render();
  x.state.active = true;
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(x.reads.length, 1, "resuming from the background does not read again");
  x.state.focused = false;
  x.state.active = false;
  x.h.render();
  x.state.focused = true;
  x.state.active = true;
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS - 300);
  assert.equal(x.reads.length, 1, "a reopened thread waits the full debounce again");
  await wait(450);
  assert.equal(x.reads.length, 2, "returning from another screen is a new open");
  x.state.epoch = 8;
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(x.reads.length, 3, "another account is a new open");
  x.h.unmount();
});

test("a pause preserves an exhausted budget while switching threads starts over, and the pending read is cancelled by a pause", async () => {
  const x = setup({ summary: exactSummary(2) });
  mountVisible(x);
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  refresh(x, exactSummary(2));
  refresh(x, exactSummary(2));
  refresh(x, exactSummary(2));
  assert.equal(x.reads.length, 3);
  x.state.active = false;
  x.h.render();
  x.state.active = true;
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(x.reads.length, 3, "a pause does not refill the budget for the same count");
  const other = `nyxa-${"d".repeat(32)}`;
  x.state.source = { type: "conversation", id: other };
  x.state.summary = { ...exactSummary(0), by_source: [{ source_type: "conversation", id: other, conversation_id: null, unread: 1 }] };
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.deepEqual(JSON.parse(JSON.stringify(x.reads.at(-1))), { source: { type: "conversation", id: other } }, "another thread reads on its own");
  x.state.source = { type: "conversation", id: C };
  x.state.summary = exactSummary(2);
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS - 300);
  x.state.active = false;
  x.h.render();
  await wait(600);
  assert.equal(x.reads.length, 4, "pausing before the debounce ends cancels that read");
  x.state.active = true;
  x.h.render();
  await wait(VISIBLE_THREAD_READ_DELAY_MS + 150);
  assert.equal(x.reads.length, 5, "coming back to the thread reads after a full debounce");
  x.h.unmount();
});
