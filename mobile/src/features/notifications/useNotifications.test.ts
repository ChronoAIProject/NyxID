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
  live?: boolean;
} = {}) {
  const state = {
    features: options.features ?? ["assistant:notifications"],
    summary: options.summary as unknown,
    dataUpdatedAt: options.dataUpdatedAt ?? 1,
    focused: options.focused ?? true,
    live: options.live ?? false,
  };
  const reads: unknown[] = [];
  const feeds: { queryKey: unknown[]; queryFn: (context: { pageParam?: string; signal?: AbortSignal }) => unknown }[] = [];
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
      useInfiniteQuery: (options: { queryKey: unknown[]; queryFn: (context: { pageParam?: string; signal?: AbortSignal }) => unknown }) => {
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
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 7 },
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
