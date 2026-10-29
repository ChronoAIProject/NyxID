import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { lightColors } from "../../theme/mobileTheme";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import {
  workspaceRecents,
  type WorkspaceRecent,
} from "./workspaceSidebarModel";

type E = { type: unknown; props: Record<string, any> };
const elements = (node: any): E[] =>
  node?.props
    ? [node, ...[node.props.children].flat(Infinity).flatMap(elements)]
    : [];
const jsx = {
  jsx: (type: unknown, props: unknown) => ({ type, props }),
  jsxs: (type: unknown, props: unknown) => ({ type, props }),
};
const native = {
  Text: "Text",
  View: "View",
  Pressable: "Pressable",
  StyleSheet: { create: (v: unknown) => v },
};
const recent = (n: number): WorkspaceRecent => ({
  key: `chat:${n}`,
  title: `Thread ${n}`,
  activityAt: "2026-10-28T00:00:00Z",
  target: {
    kind: "agent",
    agentId: "helper",
    conversationId: `nyxa-${String(n).padStart(32, "0")}`,
  },
});

test("Recents uses plain text rows, caps the sidebar at eight, and opens exact thread/See all targets", () => {
  const selected: unknown[] = [];
  let seen = 0;
  const rows = Array.from({ length: 10 }, (_, n) => recent(n));
  rows[0]!.status = "Needs you";
  const h = hookHarness<E>({
    "react/jsx-runtime": jsx,
    "react-native": native,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../lib/time": { formatRelativeTimeFromMs },
    "./useWorkspaceRecents": {
      useWorkspaceRecents: () => ({ rows, online: true }),
    },
  });
  const { WorkspaceRecents, WorkspaceRecentRow } = h.load(
    fileURLToPath(new URL("./WorkspaceRecents.tsx", import.meta.url)),
  ) as any;
  h.mount(() =>
    WorkspaceRecents({
      visible: true,
      onSelect: (target: unknown) => selected.push(target),
      onSeeAll: () => seen++,
    }),
  );
  const rowElements = elements(h.value()).filter(
    (n) => n.type === WorkspaceRecentRow,
  );
  const heading = elements(h.value()).find(
    (n) => n.props.children === "Recents",
  )!;
  const style = Object.assign({}, ...heading.props.style);
  assert.equal(style.fontSize, sidebarSectionTitle.fontSize);
  assert.equal(style.fontFamily, sidebarSectionTitle.fontFamily);
  assert.equal(style.color, lightColors.textPrimary);
  assert.equal(rowElements.length, 8);
  const row = WorkspaceRecentRow(rowElements[0]!.props);
  assert.equal(row.props.accessibilityLabel, "Open recent Thread 0, Needs you");
  assert.ok(row.props.style({ pressed: false })[0].minHeight >= 44);
  assert.ok(
    elements(row).every((n) => ["Text", "Pressable"].includes(String(n.type))),
  );
  row.props.onPress();
  assert.deepEqual(selected, [rows[0]!.target]);
  elements(h.value())
    .find((n) => n.props.accessibilityLabel === "See all recent chats")!
    .props.onPress();
  assert.equal(seen, 1);
  h.unmount();
});

test("Recents communicates empty, loading, failed and offline states without invented rows", () => {
  for (const [state, copy] of [
    [{ online: true }, "No recent chats."],
    [{ online: true, loading: true }, "Loading recent chats…"],
    [{ online: true, error: true }, "Couldn’t load recent chats."],
    [{ online: false, loading: true }, "Offline · showing saved recent chats."],
  ] as const) {
    const h = hookHarness<E>({
      "react/jsx-runtime": jsx,
      "react-native": native,
      "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
      "./useWorkspaceRecents": {
        useWorkspaceRecents: () => ({ rows: [], ...state }),
      },
    });
    const { WorkspaceRecents } = h.load(
      fileURLToPath(new URL("./WorkspaceRecents.tsx", import.meta.url)),
    ) as any;
    h.mount(() =>
      WorkspaceRecents({ visible: true, onSelect() {}, onSeeAll() {} }),
    );
    assert.ok(elements(h.value()).some((n) => n.props.children === copy));
    h.unmount();
  }
});

test("Recents shares real history/live APIs with visibility, foreground, network and account fences", async () => {
  let epoch = 5,
    online = true,
    visible = true;
  let agentId: string | undefined;
  let appChange: (value: string) => void = () => {};
  let removed = false,
    refreshed = 0,
    more = 0;
  let chatQuery: any, crewQuery: any, live: any;
  const calls: unknown[][] = [];
  const chats = {
    data: { pages: [{ conversations: [] }] },
    hasNextPage: true,
    isFetchingNextPage: false,
    isPending: false,
    refetch: async () => refreshed++,
    fetchNextPage: async () => more++,
  };
  const crews = {
    data: [],
    isPending: false,
    refetch: async () => refreshed++,
  };
  const h = hookHarness<any>({
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: (_: string, fn: typeof appChange) => {
          appChange = fn;
          return {
            remove() {
              removed = true;
            },
          };
        },
      },
    },
    "@tanstack/react-query": {
      useInfiniteQuery: (options: any) => {
        chatQuery = options;
        return chats;
      },
      useQuery: (options: any) => {
        crewQuery = options;
        return crews;
      },
    },
    "../../lib/api/assistantApi": {
      assistantApi: { list: (...args: unknown[]) => calls.push(args) },
    },
    "../../lib/api/groupsApi": {
      groupsApi: { list: (...args: unknown[]) => calls.push(args) },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: online }),
    },
    "../../features/groups/useGroupsLive": {
      useGroupsLive: (...args: unknown[]) => {
        live = args;
        return true;
      },
    },
    "../../features/groups/groupLiveRefresh": {
      groupKeys: (value: number) => ["groups", value],
    },
    "../../lib/api/assistantLiveTransport": {
      livePollInterval: (_: number, connected: boolean) =>
        connected ? false : 5000,
    },
    "./workspaceSidebarModel": { workspaceRecents },
  });
  const { useWorkspaceRecents } = h.load(
    fileURLToPath(new URL("./useWorkspaceRecents.ts", import.meta.url)),
  ) as any;
  h.mount(() => useWorkspaceRecents(visible, agentId));
  assert.deepEqual(Array.from(chatQuery.queryKey), [
    "workspace",
    "drawer-chats",
    5,
    "recents",
  ]);
  assert.deepEqual(Array.from(crewQuery.queryKey), ["groups", 5, "list"]);
  assert.equal(chatQuery.enabled, true);
  assert.equal(chatQuery.refetchInterval, false);
  assert.equal(live[2](), true);
  const signal = new AbortController().signal;
  chatQuery.queryFn({ pageParam: "older", signal });
  crewQuery.queryFn({ signal });
  assert.deepEqual(calls, [["older", undefined, signal], [signal]]);
  assert.equal(chatQuery.getNextPageParam({ next_cursor: "next" }), "next");
  const rows = h.value().rows;
  h.render();
  assert.equal(h.value().rows, rows);
  await h.value().refresh();
  h.value().loadMore();
  assert.equal(refreshed, 2);
  assert.equal(more, 1);
  agentId = "specialist";
  h.render();
  assert.equal(chatQuery.queryKey.at(-1), "specialist");
  chatQuery.queryFn({ pageParam: "agent-older", signal });
  assert.deepEqual(calls.at(-1), ["agent-older", "specialist", signal]);
  const stale = h.value();
  epoch = 6;
  await stale.refresh();
  stale.loadMore();
  assert.equal(refreshed, 2);
  assert.equal(more, 1);
  assert.equal(live[2](), false);
  h.render();
  assert.equal(chatQuery.queryKey[2], 6);
  for (const state of ["background", "active"]) {
    appChange(state);
    h.flush();
    assert.equal(chatQuery.enabled, state === "active");
  }
  online = false;
  h.render();
  await h.value().refresh();
  h.value().loadMore();
  assert.equal(chatQuery.enabled, false);
  assert.equal(h.value().online, false);
  assert.equal(refreshed, 2);
  assert.equal(more, 1);
  online = true;
  visible = false;
  h.render();
  assert.equal(crewQuery.enabled, false);
  assert.equal(live[1], false);
  h.unmount();
  assert.equal(removed, true);
});
