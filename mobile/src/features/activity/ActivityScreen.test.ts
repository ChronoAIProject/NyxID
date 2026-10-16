import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import * as challengeUiState from "./challengeUiState";
import type { ApprovalItem, ChallengeDetail } from "../../lib/api/types";

type Element = { type: unknown; props: Record<string, any> };
type FocusSlot = { callback: () => (() => void) | void; cleanup?: () => void };

function elements(root: Element): Element[] {
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
    expires_at: "2099-10-15T02:00:00Z",
    summary: "Update the DNS record",
    service_slug: "cloudflare",
    request_context: { client: "Agent", requester: "Personal agent" },
    ...overrides,
  };
}

function setup({
  challengeId,
  connected = true,
  pending = [challenge("pending-real")],
  history = [challenge("history-real", { status: "APPROVED" })],
}: {
  challengeId?: string;
  connected?: boolean;
  pending?: ChallengeDetail[];
  history?: ChallengeDetail[];
} = {}) {
  const calls: { method: string; args: any[] }[] = [];
  const mutationCalls: any[] = [];
  const titles: string[] = [];
  const activeChallenges: (string | null)[] = [];
  const events: any[] = [];
  const focusSlots: FocusSlot[] = [];
  let focusCursor = 0;
  let focused = true;
  let back: any;
  let refreshSignal: (() => void) | undefined;
  const route = { params: { challengeId } };
  const navigation = {
    setOptions: (options: { title: string }) => titles.push(options.title),
    setParams: (params: { challengeId?: string }) =>
      Object.assign(route.params, params),
  };
  const queries = new Map<string, any>();
  const makeQuery = (name: string, items: unknown[], total: number) => {
    const query = {
      data: { pages: [{ items, total, page: 1, per_page: 20 }] },
      isLoading: false,
      isPending: false,
      isError: false,
      hasNextPage: true,
      isFetchingNextPage: false,
      refetch: async (...args: unknown[]) => {
        calls.push({ method: `refresh:${name}`, args });
      },
      fetchNextPage: async () => {
        calls.push({ method: `next:${name}`, args: [] });
      },
    };
    queries.set(name, query);
    return query;
  };
  makeQuery("pending", pending, 43);
  makeQuery("history", history, 81);
  makeQuery(
    "active",
    [
      {
        id: "grant-real",
        service_name: "Cloudflare",
        expires_at: "2099-10-16T00:00:00Z",
        org_id: "org-real",
      } as ApprovalItem,
    ],
    27,
  );
  const queryClient = {
    invalidateQueries: async (args: unknown) => {
      calls.push({ method: "invalidate", args: [args] });
    },
    getQueryState: () => ({ isInvalidated: false }),
  };
  const appState = {
    currentState: "active",
    addEventListener: () => ({ remove: () => {} }),
  };
  const record =
    (method: string, value?: unknown) =>
    async (...args: any[]) => {
      calls.push({ method, args });
      return value;
    };
  const detailRefetch = record("refresh:detail");
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
          "FlatList",
          "Modal",
          "Pressable",
          "RefreshControl",
          "ScrollView",
          "Text",
          "View",
        ].map((name) => [name, name]),
      ),
      Alert: { alert: () => {} },
      AppState: appState,
      Dimensions: { get: () => ({ height: 800 }) },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "react-native-reanimated": {
      __esModule: true,
      default: { View: "AnimatedView" },
    },
    "react-native-gesture-handler": {},
    "@react-navigation/native": {
      useNavigation: () => navigation,
      useRoute: () => route,
      useFocusEffect: (callback: FocusSlot["callback"]) => {
        const index = focusCursor++;
        const old = focusSlots[index];
        if (old?.callback === callback) return;
        old?.cleanup?.();
        const slot = { callback } as FocusSlot;
        focusSlots[index] = slot;
        if (focused) slot.cleanup = callback() || undefined;
      },
    },
    "@tanstack/react-query": {
      onlineManager: { isOnline: () => connected },
      useQueryClient: () => queryClient,
      useInfiniteQuery: (options: { queryKey: string[] }) =>
        queries.get(
          options.queryKey[0] === "approvals" ? "active" : options.queryKey[1],
        ),
      useQuery: (options: { queryKey: string[]; initialData?: unknown }) =>
        options.queryKey[0] === "notifications"
          ? { data: { grant_expiry_days: 7 } }
          : { data: options.initialData, refetch: detailRefetch },
      useMutation: (config: any) => ({
        mutate: async (variables: any) => {
          mutationCalls.push(variables);
          const context = await config.onMutate?.(variables);
          try {
            const value = await config.mutationFn(variables);
            config.onSuccess?.(value, variables, context);
          } catch (error) {
            config.onError?.(error, variables, context);
          } finally {
            config.onSettled?.(undefined, undefined, variables);
          }
        },
      }),
    },
    "../../lib/telemetry": { capture: (event: unknown) => events.push(event) },
    "../../components/ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../../components/ToastOverlay": { ToastOverlay: "ToastOverlay" },
    "../../lib/notifications/pushNotifications": {
      setActiveChallengeId: (id: string | null) => activeChallenges.push(id),
    },
    "../../components/GrantCard": { GrantCard: "GrantCard" },
    "../../components/OfflineBanner": { OfflineBanner: "OfflineBanner" },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({
        isConnected: connected,
        recheckConnection: async () => connected,
      }),
    },
    "../../lib/api/mobileApi": {
      mobileApi: {
        submitDecision: record("submitDecision"),
        revoke: record("revoke"),
        getChallengeById: record(
          "getChallengeById",
          challenge(challengeId ?? "fetched-real"),
        ),
      },
    },
    "../../lib/api/idempotency": { createIdempotencyKey: () => "test-key" },
    "./challengeUiState": challengeUiState,
    "../../components/StatusBadge": { StatusBadge: "StatusBadge" },
    "../../components/PrimaryButton": { PrimaryButton: "PrimaryButton" },
    "./ApprovalEvidence": { ApprovalEvidence: "ApprovalEvidence" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../theme/flowStyles": { createFlowStyles: () => ({}) },
    "../../theme/designTokens": { radius: {}, spacing: {}, typeScale: {} },
    "../../lib/notifications/approvalRefreshSignal": {
      APPROVAL_BACKSTOP_POLL_INTERVAL_MS: 30000,
      signalApprovalStateMayHaveChanged: () => {
        refreshSignal?.();
      },
      subscribeToApprovalRefreshSignals: (callback: () => void) => {
        refreshSignal = callback;
        return () => {
          if (refreshSignal === callback) refreshSignal = undefined;
        };
      },
    },
    "lucide-react-native": { RefreshCw: "RefreshCw" },
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: (options: unknown) => {
        back = options;
      },
    },
    "./ApprovalsOverview": { ApprovalsOverview: "ApprovalsOverview" },
  });
  const { ActivityScreen } = harness.load(
    fileURLToPath(new URL("./ActivityScreen.tsx", import.meta.url)),
  ) as { ActivityScreen: () => Element };
  harness.mount(() => {
    focusCursor = 0;
    return ActivityScreen();
  });
  const find = (predicate: (item: Element) => boolean) => {
    const found = elements(harness.flush()).find(predicate);
    assert.ok(found, "Expected a matching screen element");
    return found;
  };
  return {
    harness,
    calls,
    mutationCalls,
    titles,
    activeChallenges,
    events,
    queries,
    route,
    overview: () => find((item) => item.type === "ApprovalsOverview"),
    sheet: () =>
      find(
        (item) =>
          typeof item.type === "function" &&
          item.type.name === "ChallengeDetailSheet",
      ),
    toast: () => find((item) => item.type === "ToastOverlay").props.toast,
    find,
    back: () => back,
    blur: () => {
      focused = false;
      for (const slot of focusSlots) {
        slot.cleanup?.();
        slot.cleanup = undefined;
      }
    },
    focus: () => {
      focused = true;
      for (const slot of focusSlots)
        slot.cleanup = slot.callback() || undefined;
      harness.flush();
    },
    signal: () => refreshSignal?.(),
    unmount: () => {
      for (const slot of focusSlots) slot.cleanup?.();
      harness.unmount();
    },
  };
}

test("drawer destination starts in the combined approval inbox and grant drill-in uses the native header back", () => {
  const env = setup();
  try {
    assert.equal(env.titles.at(-1), "Approvals");
    assert.equal(env.back().active, false);
    assert.equal(env.overview().props.pending.total, 43);
    assert.equal(env.overview().props.activeGrantCount, 27);
    assert.equal(env.overview().props.pending.items[0].id, "pending-real");
    assert.equal(env.overview().props.history.items[0].id, "history-real");
    env.overview().props.onOpenGrants();
    env.harness.flush();
    assert.equal(env.titles.at(-1), "Active grants");
    assert.equal(env.back().active, true);
    assert.equal(env.back().label, "Back to approvals");
    assert.equal(
      env.find((item) => item.type === "FlatList").props.data[0].id,
      "grant-real",
    );
    env.back().onBack();
    env.harness.flush();
    assert.equal(env.titles.at(-1), "Approvals");
    assert.equal(env.back().active, false);
    assert.ok(env.overview());
  } finally {
    env.unmount();
  }
});

test("reopening Approvals resets a previous grant subview while preserving a challenge deep-link sheet", async () => {
  const env = setup({ challengeId: "pending-real" });
  try {
    assert.equal(env.sheet().props.challenge.id, "pending-real");
    assert.equal(env.route.params.challengeId, undefined);
    env.overview().props.onOpenGrants();
    env.harness.flush();
    env.blur();
    env.focus();
    assert.ok(env.overview());
    assert.equal(env.back().active, false);
    assert.equal(env.sheet().props.challenge.id, "pending-real");
    assert.equal(
      env.calls.some((item) => item.method === "getChallengeById"),
      false,
    );
    assert.ok(env.activeChallenges.includes("pending-real"));
  } finally {
    env.unmount();
  }
});

test("uncached notification deep links still fetch and open the requested approval", async () => {
  const env = setup({ challengeId: "push-real" });
  try {
    await env.harness.settle();
    assert.equal(env.sheet().props.challenge.id, "push-real");
    assert.deepEqual(
      env.calls.find((item) => item.method === "getChallengeById")?.args,
      ["push-real"],
    );
    assert.equal(env.titles.at(-1), "Approvals");
  } finally {
    env.unmount();
  }
});

test("inline and detail decisions preserve request id, approval mode and server decision API", async () => {
  const grant = challenge("grant-request", { approval_mode: "grant" });
  const env = setup({ pending: [grant] });
  try {
    env.overview().props.onDecide(grant, "APPROVE");
    await env.harness.settle();
    assert.equal(env.mutationCalls[0].id, "grant-request");
    assert.equal(env.mutationCalls[0].decision, "APPROVE");
    assert.equal(env.mutationCalls[0].approvalMode, "grant");
    assert.deepEqual(
      env.calls.find((item) => item.method === "submitDecision")?.args,
      ["grant-request", "APPROVE", 7 * 86400],
    );
    assert.equal(env.toast().action.label, "View grants");
    env.toast().action.onPress();
    env.harness.flush();
    assert.equal(env.back().active, true);
    env.back().onBack();
    env.harness.flush();
    const single = challenge("single-request", {
      approval_mode: "per_request",
    });
    env.overview().props.onOpenDetail(single, "pending_list");
    env.harness.flush();
    env.sheet().props.onDeny("single-request");
    await env.harness.settle();
    assert.equal(env.mutationCalls.at(-1).id, "single-request");
    assert.equal(env.mutationCalls.at(-1).decision, "DENY");
    assert.equal(env.mutationCalls.at(-1).approvalMode, "per_request");
    assert.deepEqual(
      env.calls.filter((item) => item.method === "submitDecision").at(-1)?.args,
      ["single-request", "DENY", undefined],
    );
    assert.equal(env.toast().message, "Denied");
    assert.equal(env.toast().action, undefined);
    assert.equal(env.sheet().props.challenge, null);
  } finally {
    env.unmount();
  }
});

test("manual inbox refresh fetches pending, history and the grants total together", async () => {
  const env = setup();
  try {
    env.overview().props.onRefresh();
    await env.harness.settle();
    assert.deepEqual(
      env.calls
        .filter((item) => item.method.startsWith("refresh:"))
        .map((item) => item.method)
        .sort(),
      ["refresh:active", "refresh:history", "refresh:pending"],
    );
    env.calls.length = 0;
    env.overview().props.onPullRefresh();
    await env.harness.settle();
    assert.deepEqual(
      env.calls
        .filter((item) => item.method.startsWith("refresh:"))
        .map((item) => item.method)
        .sort(),
      ["refresh:active", "refresh:history", "refresh:pending"],
    );
    assert.equal(env.overview().props.pulling, false);
  } finally {
    env.unmount();
  }
});

test("approval signals refresh all lists from the grants subview and queue one catch-up during in-flight work", async () => {
  const env = setup();
  try {
    const gate = deferred<void>();
    const query = env.queries.get("pending");
    const refetch = query.refetch;
    let first = true;
    query.refetch = async (...args: unknown[]) => {
      await refetch(...args);
      if (first) {
        first = false;
        await gate.promise;
      }
    };
    env.harness.render();
    env.overview().props.onOpenGrants();
    env.harness.flush();
    env.signal();
    env.harness.flush();
    assert.equal(
      env.calls.filter((item) => item.method === "refresh:history").length,
      1,
    );
    env.signal();
    env.signal();
    gate.resolve();
    await env.harness.settle();
    for (const name of ["pending", "history", "active"])
      assert.equal(
        env.calls.filter((item) => item.method === `refresh:${name}`).length,
        2,
      );
    env.blur();
    env.calls.length = 0;
    env.signal();
    await env.harness.settle();
    assert.equal(env.calls.length, 0);
  } finally {
    env.unmount();
  }
});

test("pagination handlers keep pending and history separate and reject duplicate page requests", async () => {
  const env = setup();
  try {
    env.overview().props.pending.onLoadMore();
    env.overview().props.history.onLoadMore();
    await env.harness.settle();
    assert.deepEqual(
      env.calls.map((item) => item.method),
      ["next:pending", "next:history"],
    );
    env.queries.get("pending").isFetchingNextPage = true;
    env.queries.get("history").hasNextPage = false;
    env.harness.render();
    env.overview().props.pending.onLoadMore();
    env.overview().props.history.onLoadMore();
    await env.harness.settle();
    assert.equal(env.calls.length, 2);
  } finally {
    env.unmount();
  }
});

test("offline root explains connectivity and disables refresh without hiding cached approvals", async () => {
  const env = setup({ connected: false });
  try {
    assert.ok(env.find((item) => item.type === "OfflineBanner"));
    assert.equal(env.overview().props.offline, true);
    assert.equal(env.overview().props.refreshDisabled, true);
    assert.equal(env.sheet().props.isMutating, true);
    assert.equal(env.overview().props.pending.items[0].id, "pending-real");
    env.overview().props.onRefresh();
    await env.harness.settle();
    assert.equal(env.calls.length, 0);
  } finally {
    env.unmount();
  }
});

test("a paused first fetch remains pending instead of turning into a successful empty inbox", () => {
  const env = setup({ connected: false, pending: [], history: [] });
  try {
    for (const name of ["pending", "history"]) {
      const query = env.queries.get(name);
      query.data = undefined;
      query.isLoading = false;
      query.isPending = true;
    }
    env.harness.render();
    assert.equal(env.overview().props.pending.isLoading, true);
    assert.equal(env.overview().props.history.isLoading, true);
  } finally {
    env.unmount();
  }
});
