import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import type { ApprovalListState } from "./ApprovalsOverview";
import type { ChallengeDetail } from "../../lib/api/types";

type Element = { type: unknown; props: Record<string, any> };
type Section = {
  key: string;
  title: string;
  data: ChallengeDetail[];
  state: ApprovalListState;
};

function elements(root: Element | null | undefined): Element[] {
  if (!root) return [];
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

function textContent(root: Element | null | undefined): string {
  if (!root) return "";
  return [root.props.children]
    .flat(Infinity)
    .map((child) =>
      child && typeof child === "object" && "props" in child
        ? textContent(child)
        : typeof child === "string" || typeof child === "number"
          ? String(child)
          : "",
    )
    .join("");
}

function challenge(
  id: string,
  overrides: Partial<ChallengeDetail> = {},
): ChallengeDetail {
  return {
    id,
    title: "Cloudflare",
    action: "Update DNS",
    resource: "example.com",
    approval_mode: "per_request",
    risk_level: "medium",
    status: "PENDING",
    created_at: "2026-10-15T00:00:00Z",
    expires_at: "2026-10-15T02:00:00Z",
    summary: "Update the DNS record",
    service_slug: "cloudflare",
    request_context: { client: "Agent", requester: "Personal agent" },
    ...overrides,
  };
}

function list(overrides: Partial<ApprovalListState> = {}): ApprovalListState {
  return {
    items: [],
    isLoading: false,
    isError: false,
    hasNextPage: false,
    isFetchingNextPage: false,
    onRetry: () => {},
    onLoadMore: () => {},
    ...overrides,
  };
}

function setup(overrides: Record<string, unknown> = {}) {
  const callbacks: { type: string; args: unknown[] }[] = [];
  const record =
    (type: string) =>
    (...args: unknown[]) =>
      callbacks.push({ type, args });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      ...Object.fromEntries(
        [
          "ActivityIndicator",
          "Pressable",
          "RefreshControl",
          "SectionList",
          "Text",
          "View",
        ].map((name) => [name, name]),
      ),
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": {
      ChevronRight: "ChevronRight",
      RefreshCw: "RefreshCw",
      ShieldCheck: "ShieldCheck",
    },
    "../../components/ChallengeCard": { ChallengeCard: "ChallengeCard" },
    "../../components/HistoryCard": {
      HistoryCard: "HistoryCard",
      HistorySectionHeader: "HistorySectionHeader",
    },
    "../../theme/designTokens": { radius: {}, spacing: {}, typeScale: {} },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const { ApprovalsOverview, activityHistorySections } = harness.load(
    fileURLToPath(new URL("./ApprovalsOverview.tsx", import.meta.url)),
  ) as {
    ApprovalsOverview: (props: Record<string, unknown>) => Element;
    activityHistorySections: (
      items: ChallengeDetail[],
      now: Date,
    ) => { title: string; data: ChallengeDetail[] }[];
  };
  const root = harness.mount(() =>
    ApprovalsOverview({
      pending: list(),
      history: list(),
      activeGrantCount: 0,
      grantDurationLabel: "30 days",
      mutatingIds: new Set<string>(),
      refreshing: false,
      pulling: false,
      offline: false,
      refreshDisabled: false,
      onRefresh: record("refresh"),
      onPullRefresh: record("pull"),
      onOpenGrants: record("grants"),
      onOpenDetail: record("detail"),
      onDecide: record("decide"),
      ...overrides,
    }),
  );
  const sections = root.props.sections as Section[];
  const section = (key: string) => sections.find((item) => item.key === key)!;
  const header = (key: string) =>
    root.props.renderSectionHeader({ section: section(key) }) as Element;
  const footer = (key: string) =>
    root.props.renderSectionFooter({ section: section(key) }) as Element | null;
  const row = (key: string, index = 0) =>
    root.props.renderItem({
      item: section(key).data[index],
      section: section(key),
    }) as Element;
  const find = (tree: Element, label: string) => {
    const found = elements(tree).find(
      (item) => item.props.accessibilityLabel === label,
    );
    assert.ok(found, `Expected control: ${label}`);
    return found;
  };
  return {
    root,
    sections,
    section,
    header,
    footer,
    row,
    find,
    callbacks,
    activityHistorySections,
  };
}

test("one inbox contains pending and history with server totals and a secondary grants entry", () => {
  const env = setup({
    pending: list({ items: [challenge("one")], total: 43 }),
    activeGrantCount: 27,
  });
  assert.equal(env.root.type, "SectionList");
  assert.deepEqual(
    Array.from(env.sections, (item) => item.title),
    ["Incoming", "History"],
  );
  assert.match(textContent(env.header("pending")), /43/);
  assert.match(
    textContent(env.root.props.ListHeaderComponent),
    /Active grants · 27/,
  );
  env
    .find(env.root.props.ListHeaderComponent, "View active grants")
    .props.onPress();
  assert.deepEqual(env.callbacks, [{ type: "grants", args: [] }]);
  const visible =
    textContent(env.root.props.ListHeaderComponent) +
    env.sections.map((item) => textContent(env.header(item.key))).join("");
  assert.doesNotMatch(visible, /Activity|Scan|PendingActiveHistory/);
  assert.equal(
    elements(env.root.props.ListHeaderComponent).some(
      (item) => item.props.accessibilityRole === "tab",
    ),
    false,
  );
});

test("pending orders imminent expiry first and invalid or missing expiry last without mutating cached data", () => {
  const items = [
    challenge("invalid", { expires_at: "bad" }),
    challenge("late", { expires_at: "2026-10-15T03:00:00Z" }),
    challenge("missing", { expires_at: undefined as unknown as string }),
    challenge("soon", { expires_at: "2026-10-15T01:00:00Z" }),
  ];
  const env = setup({ pending: list({ items }) });
  assert.deepEqual(
    Array.from(env.section("pending").data, (item) => item.id),
    ["soon", "late", "invalid", "missing"],
  );
  assert.deepEqual(
    items.map((item) => item.id),
    ["invalid", "late", "missing", "soon"],
  );
});

test("history groups by creation day newest first, with malformed legacy dates last", () => {
  const env = setup({
    history: list({
      items: [
        challenge("invalid", { created_at: "invalid", decided_at: "invalid" }),
        challenge("old", { decided_at: "2026-10-13T01:00:00Z" }),
        challenge("fallback", {
          decided_at: "bad",
          created_at: "2026-10-14T01:00:00Z",
        }),
        challenge("latest", {
          decided_at: "2026-10-15T01:00:00Z",
          created_at: "2026-10-10T01:00:00Z",
        }),
        challenge("missing", {
          decided_at: null,
          created_at: undefined as unknown as string,
        }),
      ],
    }),
  });
  assert.ok(
    elements(env.header("history")).some(
      (n) => n.type === "HistorySectionHeader",
    ),
  );
  assert.deepEqual(
    Array.from(
      env.sections
        .filter((s) => s.key !== "pending")
        .flatMap((s) => Array.from(s.data)),
      (item) => item.id,
    ),
    ["old", "fallback", "latest", "invalid", "missing"],
  );
});

test("loading and failed first loads never claim a healthy empty approval inbox", () => {
  const loading = setup({
    pending: list({ isLoading: true }),
    history: list({ isLoading: true }),
  });
  assert.ok(
    loading.find(loading.header("pending"), "Loading pending approvals"),
  );
  assert.ok(
    loading.find(loading.header("history"), "Loading approval history"),
  );
  assert.doesNotMatch(
    textContent(loading.header("pending")),
    /Nothing waiting/,
  );
  const errors = setup({
    pending: list({ isError: true }),
    history: list({ isError: true }),
  });
  assert.match(
    textContent(errors.header("pending")),
    /Could not load pending approvals/,
  );
  assert.match(
    textContent(errors.header("history")),
    /Could not load approval history/,
  );
  assert.doesNotMatch(textContent(errors.header("pending")), /Nothing waiting/);
  assert.doesNotMatch(
    textContent(errors.header("history")),
    /Decided approvals will show/,
  );
});

test("cached requests remain readable during errors and retry stays scoped to its list", () => {
  const retries: string[] = [];
  const env = setup({
    pending: list({
      items: [challenge("cached-pending")],
      isError: true,
      onRetry: () => retries.push("pending"),
    }),
    history: list({
      items: [challenge("cached-history", { status: "APPROVED" })],
      isError: true,
      onRetry: () => retries.push("history"),
    }),
  });
  assert.match(
    textContent(env.header("pending")),
    /Displayed requests may be out of date/,
  );
  assert.match(
    textContent(env.header("history")),
    /Displayed requests may be out of date/,
  );
  assert.equal(
    elements(env.row("pending")).find((item) => item.type === "ChallengeCard")
      ?.props.challenge.id,
    "cached-pending",
  );
  assert.equal(
    elements(env.row("history")).find((item) => item.type === "HistoryCard")
      ?.props.item.id,
    "cached-history",
  );
  env.find(env.header("pending"), "Retry pending approvals").props.onPress();
  env.find(env.header("history"), "Retry approval history").props.onPress();
  assert.deepEqual(retries, ["pending", "history"]);
});

test("empty intermediate pages retain independent load-more controls", () => {
  const loads: string[] = [];
  const env = setup({
    pending: list({
      hasNextPage: true,
      onLoadMore: () => loads.push("pending"),
    }),
    history: list({
      hasNextPage: true,
      onLoadMore: () => loads.push("history"),
    }),
  });
  env
    .find(env.footer("pending")!, "Load more pending approvals")
    .props.onPress();
  env.find(env.footer("history")!, "Load older approvals").props.onPress();
  assert.deepEqual(loads, ["pending", "history"]);
  assert.equal(setup().footer("pending"), null);
  assert.equal(setup().footer("history"), null);
  const loading = setup({
    pending: list({ hasNextPage: true, isFetchingNextPage: true }),
  });
  assert.equal(loading.footer("pending")?.props.disabled, true);
});

test("pending actions and history detail callbacks preserve each real item and source", () => {
  const pending = challenge("request-real", { approval_mode: "grant" });
  const history = challenge("history-real", { status: "DENIED" });
  const env = setup({
    pending: list({ items: [pending] }),
    history: list({ items: [history] }),
  });
  const card = elements(env.row("pending")).find(
    (item) => item.type === "ChallengeCard",
  )!;
  card.props.onPress();
  card.props.onApprove();
  card.props.onDeny();
  const past = elements(env.row("history")).find(
    (item) => item.type === "HistoryCard",
  )!;
  past.props.onPress();
  assert.deepEqual(env.callbacks, [
    { type: "detail", args: [pending, "pending_list"] },
    { type: "decide", args: [pending, "APPROVE"] },
    { type: "decide", args: [pending, "DENY"] },
    { type: "detail", args: [history, "history_list"] },
  ]);
  assert.equal(past.props.onApprove, undefined);
});

test("offline and in-flight states disable mutations and requests while keeping cached detail readable", () => {
  const env = setup({
    offline: true,
    refreshDisabled: true,
    pending: list({
      items: [challenge("cached")],
      isError: true,
      hasNextPage: true,
    }),
    history: list({ isError: true, hasNextPage: true }),
  });
  assert.equal(
    env.find(env.root.props.ListHeaderComponent, "Refresh activity").props
      .disabled,
    true,
  );
  assert.equal(
    env.find(env.header("pending"), "Retry pending approvals").props.disabled,
    true,
  );
  assert.equal(
    env.find(env.header("history"), "Retry approval history").props.disabled,
    true,
  );
  assert.equal(env.footer("pending")?.props.disabled, true);
  assert.equal(env.footer("history")?.props.disabled, true);
  const card = elements(env.row("pending")).find(
    (item) => item.type === "ChallengeCard",
  )!;
  assert.equal(card.props.isMutating, false);
  assert.equal(card.props.decisionsDisabled, true);
  assert.equal(typeof card.props.onPress, "function");
  const mutating = setup({
    pending: list({ items: [challenge("busy")] }),
    mutatingIds: new Set(["busy"]),
  });
  assert.equal(
    elements(mutating.row("pending")).find(
      (item) => item.type === "ChallengeCard",
    )?.props.isMutating,
    true,
  );
});

test("manual refresh and pull-to-refresh remain separate signals and unknown grant totals stay unknown", () => {
  const env = setup({
    activeGrantCount: undefined,
    refreshing: true,
    pulling: false,
  });
  assert.match(
    textContent(env.root.props.ListHeaderComponent),
    /Active grants/,
  );
  assert.doesNotMatch(
    textContent(env.root.props.ListHeaderComponent),
    /Active grants · 0/,
  );
  const refresh = env.find(
    env.root.props.ListHeaderComponent,
    "Refresh activity",
  );
  assert.equal(refresh.props.accessibilityState.disabled, false);
  assert.equal(refresh.props.accessibilityState.busy, true);
  assert.equal(env.root.props.refreshControl.props.refreshing, false);
  refresh.props.onPress();
  env.root.props.refreshControl.props.onRefresh();
  assert.deepEqual(env.callbacks, [
    { type: "refresh", args: [] },
    { type: "pull", args: [] },
  ]);
});

test("an offline inbox without cached rows asks to connect rather than claiming nothing is pending", () => {
  const env = setup({ offline: true });
  assert.match(
    textContent(env.header("pending")),
    /Connect to check pending approvals/,
  );
  assert.doesNotMatch(
    textContent(env.header("pending")),
    /Nothing waiting on you/,
  );
  assert.match(
    textContent(env.header("history")),
    /Connect to check approval history/,
  );
  assert.doesNotMatch(
    textContent(env.header("history")),
    /Decided approvals will show up here/,
  );
});

test("Activity history uses main's Today/Yesterday/date labels and paginates only after the last date", () => {
  const env = setup({
    history: list({
      hasNextPage: true,
      items: [
        challenge("old", { created_at: "2026-10-01T12:00:00" }),
        challenge("today", { created_at: "2026-10-28T12:00:00" }),
        challenge("yesterday", { created_at: "2026-10-27T12:00:00" }),
      ],
    }),
  });
  const dates = env.activityHistorySections(
    [
      challenge("today", { created_at: "2026-10-28T12:00:00" }),
      challenge("yesterday", { created_at: "2026-10-27T12:00:00" }),
      challenge("old", { created_at: "2026-10-01T12:00:00" }),
    ],
    new Date("2026-10-28T18:00:00"),
  );
  assert.deepEqual(
    Array.from(dates, (g) => g.title),
    ["Today", "Yesterday", "Oct 1"],
  );
  assert.equal(env.footer("history"), null);
  assert.ok(env.footer(env.sections.at(-1)!.key));
});

test("Activity owns its content inset below the opaque slab header", () => {
  const env = setup();
  assert.equal(env.root.props.contentInsetAdjustmentBehavior, "never");
  assert.equal(env.root.props.automaticallyAdjustContentInsets, false);
  assert.equal(env.root.props.contentContainerStyle.paddingTop, 16);
  assert.equal(env.root.props.stickySectionHeadersEnabled, false);
});
