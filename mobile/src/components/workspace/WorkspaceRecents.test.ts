import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { lightColors } from "../../theme/mobileTheme";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import {
  recentUnread,
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
let fontScale = 1;
const native = {
  Text: "Text",
  View: "View",
  Pressable: "Pressable",
  StyleSheet: { create: (v: unknown) => v },
  useWindowDimensions: () => ({ width: 466, height: 678, fontScale, scale: 3 }),
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
    "../Skeleton": { SkeletonRows: "SkeletonRows" },
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
    // Plain text rows: labels plus the one View grouping count, status, time.
    elements(row).every((n) =>
      ["Text", "Pressable", "View"].includes(String(n.type)),
    ),
  );
  row.props.onPress();
  assert.deepEqual(selected, [rows[0]!.target]);
  elements(h.value())
    .find((n) => n.props.accessibilityLabel === "See all recent chats")!
    .props.onPress();
  assert.equal(seen, 1);
  h.unmount();
});

test("on the Duo at accessibility text sizes a recent row keeps the title's full width and puts count, status and time beneath it", () => {
  const corner = { value: true };
  const h = hookHarness<E>({
    "./WorkspaceLayoutContext": { WorkspaceCornerSensorContext: corner },
    "react/jsx-runtime": jsx,
    "react-native": native,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../lib/time": { formatRelativeTimeFromMs },
    "../Skeleton": { SkeletonRows: "SkeletonRows" },
    "./useWorkspaceRecents": {
      useWorkspaceRecents: () => ({ rows: [], online: true }),
    },
  });
  const { WorkspaceRecentRow, RECENT_ROW_STACK_FONT_SCALE } = h.load(
    fileURLToPath(new URL("./WorkspaceRecents.tsx", import.meta.url)),
  ) as any;
  const item = {
    ...recent(1),
    title: "Reviewing the Duo layout",
    status: "Needs you",
  };
  const flat = (style: unknown) =>
    Object.assign({}, ...[style].flat(Infinity).filter(Boolean));
  const render = (scale: number) => {
    fontScale = scale;
    // Rendered again on a live size change: the row re-reads fontScale.
    h.mount(() => WorkspaceRecentRow({ item, unread: 2, onSelect() {} }));
    const row = h.value();
    const texts = elements(row).filter((n) => n.type === "Text");
    const [title, count, status, time] = texts.map((n) => flat(n.props.style));
    const meta = flat(
      elements(row).find((n) => n.props.testID === "recent-row-meta")!.props
        .style,
    );
    h.unmount();
    return {
      row: flat(row.props.style({ pressed: false })),
      texts,
      title,
      count,
      status,
      time,
      meta,
    };
  };
  try {
    // XXXL, the largest non-accessibility size, keeps today's single line.
    assert.ok(
      RECENT_ROW_STACK_FONT_SCALE > 1.353 &&
        RECENT_ROW_STACK_FONT_SCALE <= 1.786,
    );
    for (const scale of [1, 1.353]) {
      const one = render(scale);
      assert.equal(one.row.flexDirection, "row", String(scale));
      assert.equal(one.title.flex, 1);
      assert.equal(one.meta.flexDirection, "row");
      assert.equal(one.status.maxWidth, 78);
    }
    for (const scale of [1.786, 3.571]) {
      const stacked = render(scale);
      // A column: the title spans the row's width on its own line (so it
      // ellipsizes at the row's edge) and the meta sits beneath it...
      assert.equal(stacked.row.flexDirection, "column", String(scale));
      assert.equal(stacked.row.alignItems, "stretch");
      assert.equal(stacked.title.flex, 0);
      assert.equal(stacked.meta.flexDirection, "row");
      assert.equal(stacked.meta.flexWrap, "wrap");
      // ...where the status is no longer held to its compact width.
      assert.equal(stacked.status.maxWidth, "100%");
      // The row grows with its text: no fixed height anywhere.
      assert.equal(stacked.row.height, undefined);
      assert.ok(stacked.row.minHeight >= 44);
      assert.equal(stacked.time.flexShrink, 1);
      // Strict no-wrap: every label is still one line with a tail ellipsis.
      for (const text of [stacked.texts[0]!, stacked.texts.at(-1)!])
        assert.equal(text.props.numberOfLines, 1);
      assert.deepEqual(
        stacked.texts.map((n) => [n.props.children].flat().join("")),
        [
          "Reviewing the Duo layout",
          "2",
          "Needs you",
          formatRelativeTimeFromMs(Date.parse(item.activityAt)),
        ],
      );
    }
    corner.value = false;
    const ordinary = render(3.571);
    assert.equal(
      ordinary.row.flexDirection,
      "row",
      "ordinary devices retain base layout at large text",
    );
    assert.equal(ordinary.status.maxWidth, 78);
  } finally {
    fontScale = 1;
  }
});

test("Recents communicates empty, loading, failed and offline states without invented rows", () => {
  for (const [state, copy] of [
    [{ online: true }, "No recent chats."],
    [{ online: true, loading: true }, "skeleton:Loading recent chats"],
    [{ online: true, error: true }, "Couldn’t load recent chats."],
    [{ online: false, loading: true }, "Offline · showing saved recent chats."],
  ] as const) {
    const h = hookHarness<E>({
      "react/jsx-runtime": jsx,
      "react-native": native,
      "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
      "../Skeleton": { SkeletonRows: "SkeletonRows" },
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
    // A first load is a skeleton of rows, never "Loading…" text.
    assert.ok(
      elements(h.value()).some((n) =>
        copy.startsWith("skeleton:")
          ? n.type === "SkeletonRows" &&
            n.props.accessibilityLabel === copy.slice("skeleton:".length)
          : n.props.children === copy,
      ),
      copy,
    );
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

test("Recents rows show the unread counts the summary attributes to their thread or crew", () => {
  const rows = [
    recent(0),
    recent(1),
    {
      ...recent(2),
      key: "crew:g",
      title: "Launch crew",
      target: { kind: "group" as const, groupId: `nyxg-${"e".repeat(32)}` },
    },
  ];
  const h = hookHarness<E>({
    "react/jsx-runtime": jsx,
    "react-native": native,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../lib/time": { formatRelativeTimeFromMs },
    "../Skeleton": { SkeletonRows: "SkeletonRows" },
    "./useWorkspaceRecents": {
      useWorkspaceRecents: () => ({ rows, online: true }),
    },
  });
  const { WorkspaceRecents, WorkspaceRecentRow } = h.load(
    fileURLToPath(new URL("./WorkspaceRecents.tsx", import.meta.url)),
  ) as any;
  const unread = {
    conversations: { [`nyxa-${"1".padStart(32, "0")}`]: 4 },
    groups: { [`nyxg-${"e".repeat(32)}`]: 120 },
  };
  h.mount(() =>
    WorkspaceRecents({ visible: true, unread, onSelect() {}, onSeeAll() {} }),
  );
  const rendered = elements(h.value())
    .filter((n) => n.type === WorkspaceRecentRow)
    .map((n) => WorkspaceRecentRow(n.props));
  assert.deepEqual(
    rendered.map((row) => row.props.accessibilityLabel),
    [
      "Open recent Thread 0",
      "Open recent Thread 1, 4 unread",
      "Open recent Launch crew, 120 unread",
    ],
  );
  const pills = rendered.map((row) =>
    elements(row).find((n) => n.props.testID === "sidebar-unread-count"),
  );
  assert.equal(pills[0], undefined);
  assert.equal(pills[1]!.props.children, "4");
  assert.equal(pills[2]!.props.children, "99+");
  assert.equal(recentUnread({ target: { kind: "home" } }, unread), 0);
  assert.equal(
    recentUnread({ target: { kind: "agent", agentId: "helper" } }, unread),
    0,
    "an agent target without a thread has no marker",
  );
  h.unmount();
  const bare = hookHarness<E>({
    "react/jsx-runtime": jsx,
    "react-native": native,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../lib/time": { formatRelativeTimeFromMs },
    "../Skeleton": { SkeletonRows: "SkeletonRows" },
    "./useWorkspaceRecents": {
      useWorkspaceRecents: () => ({ rows, online: true }),
    },
  });
  const plain = bare.load(
    fileURLToPath(new URL("./WorkspaceRecents.tsx", import.meta.url)),
  ) as any;
  bare.mount(() =>
    plain.WorkspaceRecents({ visible: true, onSelect() {}, onSeeAll() {} }),
  );
  const plainRows = elements(bare.value())
    .filter((n) => n.type === plain.WorkspaceRecentRow)
    .map((n) => plain.WorkspaceRecentRow(n.props));
  assert.ok(
    plainRows.every(
      (row) =>
        !elements(row).some((n) => n.props.testID === "sidebar-unread-count"),
    ),
    "no markers without a summary",
  );
  bare.unmount();
});

test("recent rows and See all disable with muted text during a critical write and recover without remounting", () => {
  type RecentProps = Parameters<
    typeof import("./WorkspaceRecents").WorkspaceRecents
  >[0];
  type RowProps = Parameters<
    typeof import("./WorkspaceRecents").WorkspaceRecentRow
  >[0];
  const h = hookHarness<E>({
    "react/jsx-runtime": jsx,
    "react-native": native,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../lib/time": { formatRelativeTimeFromMs },
    "../Skeleton": { SkeletonRows: "SkeletonRows" },
    "./useWorkspaceRecents": {
      useWorkspaceRecents: () => ({
        rows: [{ ...recent(0), status: "Needs you" }],
        online: true,
      }),
    },
  });
  const { WorkspaceRecents, WorkspaceRecentRow } = h.load(
    fileURLToPath(new URL("./WorkspaceRecents.tsx", import.meta.url)),
  ) as {
    WorkspaceRecents: (props: RecentProps) => E;
    WorkspaceRecentRow: (props: RowProps) => E;
  };
  let selected = 0;
  const props: RecentProps = {
    visible: true,
    disabled: true,
    onSelect: () => {
      selected++;
    },
    onSeeAll() {},
  };
  h.mount(() => WorkspaceRecents(props));
  const rows = () =>
    elements(h.value())
      .filter((node) => node.type === WorkspaceRecentRow)
      .map((node) => WorkspaceRecentRow(node.props as RowProps));
  const seeAll = () =>
    elements(h.value()).find(
      (node) => node.props.accessibilityLabel === "See all recent chats",
    )!;
  for (const control of [...rows(), seeAll()]) {
    assert.equal(control.props.disabled, true);
    assert.equal(control.props.accessibilityState.disabled, true);
    const style = Object.assign(
      {},
      ...[
        typeof control.props.style === "function"
          ? control.props.style({ pressed: false })
          : control.props.style,
      ].flat(Infinity),
    );
    assert.ok(style.minHeight >= 44);
    for (const label of elements(control).filter(
      (node) => node.type === "Text",
    )) {
      const textStyle = Object.assign(
        {},
        ...[label.props.style].flat(Infinity).filter(Boolean),
      );
      assert.equal(textStyle.color, lightColors.textMuted);
      assert.equal(textStyle.opacity, undefined);
    }
  }
  props.disabled = false;
  h.render();
  assert.equal(rows()[0]!.props.disabled, false);
  assert.equal(seeAll().props.disabled, false);
  rows()[0]!.props.onPress();
  assert.equal(selected, 1);
  h.unmount();
});
