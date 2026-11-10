import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { lightColors } from "../../theme/mobileTheme";
import { notificationSchema, type NotificationFilter } from "./notificationContracts";

type E = { type: unknown; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const text = (n: any): string =>
  typeof n === "string" || typeof n === "number"
    ? String(n)
    : n?.props
      ? [n.props.children].flat(Infinity).map(text).join(" ")
      : "";
const C = `nyxa-${"a".repeat(32)}`;
const G = `nyxg-${"b".repeat(32)}`;
const T = "7f6b2b3c-1c4a-4c33-9d1e-0a1b2c3d4e5f";
const AS_OF = "2026-11-10T08:06:00Z";
const row = (id: string, overrides: Record<string, unknown>) =>
  notificationSchema.parse({
    id,
    kind: "agent_update",
    source: { type: "conversation", id: C },
    created_at: "2026-11-10T08:00:00Z",
    updated_at: "2026-11-10T08:05:00Z",
    ...overrides,
  });

function setup(options: {
  enabled?: boolean;
  pages?: unknown[];
  pending?: boolean;
  error?: Error;
  hasNextPage?: boolean;
  summary?: unknown;
} = {}) {
  const reads: unknown[] = [];
  const opened: unknown[] = [];
  const pushed: unknown[] = [];
  const filters: NotificationFilter[] = [];
  const fetched: string[] = [];
  let header: { label: string; enabled: boolean; busy: boolean; onPress: () => void } | undefined;
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      ActivityIndicator: "ActivityIndicator",
      FlatList: "FlatList",
      Pressable: "Pressable",
      RefreshControl: "RefreshControl",
      View: "View",
      StyleSheet: { create: (v: unknown) => v, hairlineWidth: 0.5 },
    },
    "lucide-react-native": new Proxy({}, { get: (_t, name) => (name === "__esModule" ? false : String(name)) }),
    "../../app/WorkspaceHeader": {
      useOpenWorkspaceChat: () => (params: unknown) => opened.push(params),
      useWorkspaceBack: () => {},
    },
    "../../components/DynamicTypeText": { DynamicTypeText: "Text" },
    "../../components/SegmentControl": { SegmentControl: "SegmentControl" },
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsRow: "SettingsRow",
      useSettingsHeaderAction: (_navigation: unknown, value: typeof header) => {
        header = value;
      },
    },
    "../../components/workspace/WorkspaceUI": { Notice: "Notice", WorkspacePage: "WorkspacePage" },
    "../../hooks/usePullRefresh": {
      usePullRefresh: (refetch: () => unknown) => ({ refreshing: false, onRefresh: refetch }),
    },
    "../../lib/env": { firstPartyFrontendUrls: () => ["https://app.nyxid.test"] },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "./useNotifications": {
      useNotificationRuntime: () => ({ active: true, online: true }),
      useNotificationsEnabled: () => options.enabled ?? true,
      useNotificationsLive: () => false,
      useNotificationFeed: (filter: NotificationFilter) => {
        filters.push(filter);
        return {
          data: options.pages ? { pages: options.pages } : undefined,
          isPending: options.pending ?? false,
          isError: !!options.error,
          error: options.error,
          hasNextPage: options.hasNextPage ?? false,
          isFetchingNextPage: false,
          fetchNextPage: () => {
            fetched.push(filter);
            return Promise.resolve();
          },
          refetch: () => Promise.resolve(),
        };
      },
      useNotificationSummary: () => ({ data: options.summary, refetch: () => Promise.resolve() }),
      useMarkNotificationsRead: () => ({ mutate: (scope: unknown) => reads.push(scope), isPending: false }),
    },
  });
  const module = h.load(fileURLToPath(new URL("./NotificationsScreen.tsx", import.meta.url))) as Record<string, any>;
  const navigation = { goBack: () => {}, push: (...args: unknown[]) => pushed.push(args), setOptions: () => {} };
  // Our own function components (rows, chips) render inline so the tree holds real rows.
  const expand = (node: any): any => {
    if (Array.isArray(node)) return node.map(expand);
    if (!node || typeof node !== "object" || !("props" in node)) return node;
    if (typeof node.type === "function") return expand(node.type(node.props));
    const props = typeof node.props?.style === "function" ? { ...node.props, style: node.props.style({ pressed: false }) } : node.props;
    const children = expand(props?.children);
    const next = { ...node, props: { ...props, children } };
    if (next.props.ListEmptyComponent) next.props.ListEmptyComponent = expand(next.props.ListEmptyComponent);
    return next;
  };
  h.mount(() => expand(module.NotificationsScreen({ navigation, route: { params: undefined } })));
  const all = () => nodes(h.value());
  const list = () => all().find((n) => n.type === "FlatList")!;
  const renderedRows = () => (list().props.data as any[]).map((item) => expand(list().props.renderItem({ item })));
  return { h, module, all, list, renderedRows, reads, opened, pushed, filters, fetched, header: () => header! };
}

test("rows show title, preview, counts and unread state; tapping marks read and opens the thread", () => {
  const x = setup({
    pages: [
      {
        as_of: AS_OF,
        next_cursor: null,
        items: [
          row("n1", { kind: "chat_update", title: "Support chat", preview: "Thanks!", count: 3,
            source: { type: "channel_chat", id: C, channel_id: "ch", agent_id: "agent-one" } }),
          row("n2", { kind: "needs_decision", title: "Delete repo?", read: true, source: { type: "conversation", id: C } }),
          row("n3", { kind: "group_mention", read: true, source: { type: "group", id: G } }),
          row("n4", { kind: "automation_problem", title: "Digest", source: { type: "trigger", id: T } }),
        ],
      },
    ],
    summary: { unread_count: 2, needs_decision_count: 1, by_source: [], by_source_truncated: false },
  });
  const rows = x.renderedRows();
  assert.equal(rows.length, 4);
  const first = nodes(rows[0]);
  assert.ok(first.some((n) => n.type === "Text" && n.props.children === "Support chat"));
  assert.ok(first.some((n) => n.type === "Text" && n.props.children === "Thanks!"));
  assert.ok(first.some((n) => n.type === "Text" && n.props.children === "3 new"));
  assert.ok(first.some((n) => n.props.testID === "notification-unread-dot"));
  assert.match(rows[0].props.accessibilityLabel, /^Support chat, Thanks!, .*3 new, unread$/);
  const decision = nodes(rows[1]);
  assert.ok(decision.some((n) => n.type === "Text" && n.props.children === "Needs you"));
  assert.equal(decision.some((n) => n.props.testID === "notification-unread-dot"), false, "a seen card has no unread dot");
  rows[0].props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(x.reads)), [{ ids: ["n1"] }]);
  assert.deepEqual(JSON.parse(JSON.stringify(x.opened)), [
    { conversationId: C, agentId: "agent-one", home: false, draft: false, openLatest: false },
  ]);
  rows[1].props.onPress();
  assert.equal(x.reads.length, 1, "an already-read row is not marked again");
  assert.equal(x.opened.length, 2);
  rows[2].props.onPress();
  rows[3].props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(x.pushed)), [["GroupChat", { groupId: G }], ["AutomationDetail", { id: T }]]);
  assert.deepEqual(JSON.parse(JSON.stringify(x.reads.at(-1))), { ids: ["n4"] });
  x.h.unmount();
});

test("rows follow their canonical link: an organization bot's public transcript opens the agent's details, never a hidden thread", () => {
  const AGENT = "7c0f6a8e-2b2e-4a1a-9f1f-0d2c5a6b7c8d";
  const x = setup({
    pages: [
      {
        as_of: AS_OF,
        next_cursor: null,
        items: [
          row("n1", { kind: "chat_update", title: "Sales room",
            source: { type: "channel_chat", id: "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", channel_id: "ch", agent_id: AGENT },
            url: `https://app.nyxid.test/assistant?agent=${AGENT}&details=channels` }),
          row("n2", { kind: "chat_update", title: "Elsewhere",
            source: { type: "channel_chat", id: "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", channel_id: "ch", agent_id: AGENT },
            url: `https://evil.example/assistant?agent=${AGENT}&details=channels` }),
          row("n3", { kind: "needs_decision", source: { type: "conversation", id: C }, url: `https://app.nyxid.test/assistant?c=${C}&card=card-2` }),
          row("n4", { kind: "automation_problem", source: { type: "trigger", id: "legacy", agent_id: AGENT }, url: `https://app.nyxid.test/assistant/automations?agent=${AGENT}` }),
          row("n5", { kind: "group_mention", source: { type: "group", id: G }, url: `https://app.nyxid.test/assistant?c=${G}` }),
        ],
      },
    ],
    summary: { unread_count: 5, needs_decision_count: 1, by_source: [], by_source_truncated: false },
  });
  const rows = x.renderedRows();
  rows[0].props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(x.pushed)), [["AgentDetail", { agentId: AGENT }]]);
  rows[1].props.onPress();
  assert.equal(x.pushed.length, 1, "a link on another origin opens nothing");
  assert.equal(x.opened.length, 0, "and the hidden thread id never becomes a chat target");
  assert.deepEqual(JSON.parse(JSON.stringify(x.reads)), [{ ids: ["n1"] }, { ids: ["n2"] }], "both taps still mark their row read");
  rows[2].props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(x.opened)), [
    { conversationId: C, acknowledgementId: "card-2", home: false, draft: false, openLatest: false },
  ]);
  rows[3].props.onPress();
  rows[4].props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(x.pushed.slice(1))), [["Automations", { agentId: AGENT }], ["GroupChat", { groupId: G }]]);
  x.h.unmount();
});

test("mark all read uses the server fence, filters re-query by kind and the end of the list loads more", () => {
  const x = setup({
    pages: [{ as_of: AS_OF, next_cursor: "more", items: [row("n1", {})] }],
    hasNextPage: true,
    summary: { unread_count: 1, needs_decision_count: 2, by_source: [], by_source_truncated: false },
  });
  const header = x.header();
  assert.equal(header.label, "Mark all notifications as read");
  assert.equal(header.enabled, true);
  header.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(x.reads)), [{ all: true, before: AS_OF }]);
  const segments = x.all().find((n) => n.type === "SegmentControl")!;
  assert.deepEqual(segments.props.segments.map((s: any) => s.label), ["All", "Needs you", "Automations", "Chats"]);
  assert.equal(segments.props.segments[1].count, 2);
  assert.equal(segments.props.activeIndex, 0);
  segments.props.onPress(1);
  x.h.flush();
  assert.equal(x.filters.at(-1), "needs_you");
  assert.equal(x.all().find((n) => n.type === "SegmentControl")!.props.activeIndex, 1);
  // The agent_update row leaves Needs you empty while a cursor remains, so the
  // list walks on by itself once; the end of the list then asks for more.
  assert.deepEqual(x.fetched, ["needs_you"]);
  x.list().props.onEndReached();
  assert.deepEqual(x.fetched, ["needs_you", "needs_you"]);
  x.h.unmount();
  const none = setup({ pages: [{ as_of: AS_OF, next_cursor: null, items: [] }], summary: { unread_count: 0, needs_decision_count: 0, by_source: [], by_source_truncated: false } });
  assert.equal(none.header().enabled, false, "nothing to mark");
  assert.ok(nodes(none.list().props.ListEmptyComponent).some((n) => n.type === "Text" && n.props.children === "You're all caught up."));
  assert.deepEqual(none.fetched, [], "an exhausted feed asks for nothing more");
  none.h.unmount();
});

test("the flag, loading and failures each have their own state, and resolved cards leave Needs you", () => {
  const off = setup({ enabled: false });
  const notice = nodes(off.list().props.ListEmptyComponent).find((n) => n.type === "Notice")!;
  assert.equal(notice.props.message, "Notifications are not available for this account yet.");
  off.h.unmount();
  const loading = setup({ pending: true });
  assert.ok(nodes(loading.list().props.ListEmptyComponent).some((n) => n.type === "SkeletonRows"));
  loading.h.unmount();
  const failed = setup({ error: new Error("Boom") });
  const empty = nodes(failed.list().props.ListEmptyComponent);
  assert.equal(empty.find((n) => n.type === "Notice")!.props.message, "Boom");
  assert.equal(empty.find((n) => n.type === "Notice")!.props.tone, "error");
  assert.ok(empty.some((n) => n.type === "SettingsRow" && n.props.label === "Retry"));
  failed.h.unmount();
  const pages = [
    { as_of: AS_OF, next_cursor: null, items: [
      row("n1", { kind: "needs_decision", resolved: true }),
      row("n2", { kind: "needs_decision", resolved: false }),
      row("n2", { kind: "needs_decision", resolved: false }),
    ] },
  ];
  const all = setup({ pages });
  assert.deepEqual(Array.from(all.list().props.data, (item: any) => item.id), ["n1", "n2"], "duplicates across pages collapse");
  const keep = all.module.notificationRows(pages, "needs_you");
  assert.deepEqual(Array.from(keep, (item: any) => item.id), ["n2"]);
  all.h.unmount();
});

test("an empty page with a cursor keeps paging on its own, stops on a repeated cursor and offers Load more, and ends only when exhausted", () => {
  const options: {
    pages: { as_of: string; next_cursor: string | null; items: unknown[] }[];
    hasNextPage: boolean;
    summary: unknown;
  } = {
    pages: [{ as_of: AS_OF, next_cursor: "p2", items: [] }],
    hasNextPage: true,
    summary: { unread_count: 1, needs_decision_count: 0, by_source: [], by_source_truncated: false },
  };
  const x = setup(options);
  x.h.flush();
  assert.deepEqual(x.fetched, ["all"], "the empty first page is followed automatically");
  const emptyNodes = () => nodes(x.list().props.ListEmptyComponent);
  assert.ok(emptyNodes().some((n) => n.type === "SkeletonRows" && n.props.accessibilityLabel === "Looking for updates"));
  assert.equal(emptyNodes().some((n) => n.type === "Text" && n.props.children === "You're all caught up."), false, "not exhausted yet");
  x.h.render();
  x.h.flush();
  assert.deepEqual(x.fetched, ["all"], "the same cursor is never followed twice");
  options.pages.push({ as_of: AS_OF, next_cursor: "p2", items: [] });
  x.h.render();
  x.h.flush();
  assert.deepEqual(x.fetched, ["all"], "an unchanged cursor stops the automatic walk");
  const loadMore = emptyNodes().find((n) => n.type === "SettingsRow" && n.props.label === "Load more");
  assert.ok(loadMore, "pagination stays reachable by hand");
  loadMore!.props.onPress();
  assert.deepEqual(x.fetched, ["all", "all"]);
  options.pages.push({ as_of: AS_OF, next_cursor: "p3", items: [] });
  x.h.render();
  x.h.flush();
  assert.deepEqual(x.fetched, ["all", "all", "all"], "a new cursor is followed again");
  options.pages.push({ as_of: AS_OF, next_cursor: null, items: [row("n9", { kind: "chat_update", title: "Found" })] });
  options.hasNextPage = false;
  x.h.render();
  x.h.flush();
  assert.equal(x.renderedRows().length, 1, "the visible row from a later page is reachable");
  assert.deepEqual(x.fetched, ["all", "all", "all"]);
  x.h.unmount();
  const hops = setup({ pages: [{ as_of: AS_OF, next_cursor: "c1", items: [] }], hasNextPage: true,
    summary: { unread_count: 0, needs_decision_count: 0, by_source: [], by_source_truncated: false } });
  assert.equal(hops.module.AUTO_EMPTY_PAGE_HOPS, 5);
  assert.equal(hops.module.nextEmptyPageHop("c6", ["c1", "c2", "c3", "c4", "c5"]), false, "the automatic walk is bounded");
  assert.equal(hops.module.nextEmptyPageHop("c2", ["c1"]), true);
  assert.equal(hops.module.nextEmptyPageHop(null, []), false);
  hops.h.unmount();
});
