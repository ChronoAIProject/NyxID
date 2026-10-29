import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness, deferred } from "../assistant/assistantHookTestHarness";
import { darkColors } from "../../theme/mobileTheme";
import * as challengeUiState from "./challengeUiState";

type Element = { type: unknown; props: Record<string, any> };
const nodes = (e: Element): Element[] => [
  e,
  ...[e.props.children]
    .flat(Infinity)
    .flatMap((c) => (c?.props ? nodes(c) : [])),
];
function setup() {
  const calls: unknown[][] = [];
  let guard: any;
  let pending = false;
  const gate = deferred<void>();
  const data = {
    id: "request-1",
    title: "GitHub",
    action: "Write",
    resource: "repo",
    summary: "Review write",
    approval_mode: "grant",
    risk_level: "low",
    status: "PENDING",
    created_at: "2026-10-28T00:00:00Z",
    expires_at: "2099-10-28T00:00:00Z",
    request_context: { client: "agent", requester: "reviewer" },
  };
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      ...Object.fromEntries(
        ["Pressable", "ScrollView", "Text", "View"].map((n) => [n, n]),
      ),
      AppState: { currentState: "active" },
      StyleSheet: { create: (s: unknown) => s },
    },
    "@react-navigation/native": { useFocusEffect: () => {} },
    "@tanstack/react-query": {
      onlineManager: { isOnline: () => true },
      useQueryClient: () => ({ invalidateQueries: async () => {} }),
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        data: queryKey[0] === "notifications" ? { grant_expiry_days: 7 } : data,
      }),
      useMutation: (config: any) => ({
        isPending: pending,
        mutate: async (decision: string) => {
          pending = true;
          h.render();
          await config.mutationFn(decision);
          pending = false;
          h.render();
        },
      }),
    },
    "../../components/ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../../components/FullScreenLoading": {
      FullScreenLoading: "FullScreenLoading",
    },
    "../../components/ToastOverlay": { ToastOverlay: "ToastOverlay" },
    "../../components/StatusBadge": { StatusBadge: "StatusBadge" },
    "../../components/PrimaryButton": { PrimaryButton: "PrimaryButton" },
    "./ApprovalEvidence": { ApprovalEvidence: "ApprovalEvidence" },
    "../../lib/telemetry": { capture: () => {} },
    "../../lib/notifications/pushNotifications": {
      setActiveChallengeId: () => {},
    },
    "../../lib/api/mobileApi": {
      mobileApi: {
        submitDecision: async (...args: unknown[]) => {
          calls.push(args);
          await gate.promise;
        },
      },
    },
    "./challengeUiState": challengeUiState,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: darkColors }) },
    "../../theme/designTokens": { radius: {}, spacing: {}, typeScale: {} },
    "../../theme/flowStyles": { createFlowStyles: () => ({}) },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: true }),
    },
    "../../lib/notifications/approvalRefreshSignal": {
      APPROVAL_BACKSTOP_POLL_INTERVAL_MS: 5000,
    },
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: (v: unknown) => {
        guard = v;
      },
    },
  });
  const screen = h.load(
    fileURLToPath(new URL("./ActivityDetailScreen.tsx", import.meta.url)),
  ).ActivityDetailScreen as (props: unknown) => Element;
  h.mount(() =>
    screen({
      route: { params: { challengeId: data.id }, key: "detail" },
      navigation: { goBack: () => {}, isFocused: () => true },
    }),
  );
  return {
    h,
    calls,
    gate,
    guard: () => guard,
    button: (label: string) =>
      nodes(h.flush()).find((n) => n.props.label === label)!,
  };
}

test("the native approval detail page preserves grant duration and blocks Back during the real decision promise", async () => {
  const e = setup();
  assert.equal(e.guard().defaultRole, "back");
  assert.equal(e.guard().disabled, false);
  const request = e.button("Approve").props.onPress();
  e.h.flush();
  assert.deepEqual(e.calls, [["request-1", "APPROVE", 7 * 86400]]);
  assert.equal(e.guard().disabled, true);
  e.gate.resolve();
  await request;
  e.h.flush();
  assert.equal(e.guard().disabled, false);
  e.h.unmount();
});

test("denial on the native detail page sends no reusable grant duration", async () => {
  const e = setup();
  const request = e.button("Deny").props.onPress();
  assert.deepEqual(e.calls, [["request-1", "DENY", undefined]]);
  e.gate.resolve();
  await request;
  e.h.unmount();
});
