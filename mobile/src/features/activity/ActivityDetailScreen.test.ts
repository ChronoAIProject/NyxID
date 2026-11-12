import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness, deferred } from "../assistant/assistantHookTestHarness";
import { darkColors } from "../../theme/mobileTheme";
import * as challengeUiState from "./challengeUiState";
import type { ChallengeDetail } from "../../lib/api/types";

type Element = { type: unknown; props: Record<string, any> };
const nodes = (e: Element): Element[] => [
  e,
  ...[e.props.children]
    .flat(Infinity)
    .flatMap((c) => (c?.props ? nodes(c) : [])),
];
function setup(execution_proposal?: ChallengeDetail["execution_proposal"]) {
  const calls: unknown[][] = [];
  const openedUrls: string[] = [];
  let frontendUrl = "https://app.example.test";
  let guard: any;
  let pending = false;
  const gate = deferred<void>();
  const data = {
    execution_proposal,
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
    "../../lib/env": { getFrontendUrl: () => frontendUrl },
    "react-native": {
      Linking: { openURL: async (url: string) => { openedUrls.push(url); } },
      ...Object.fromEntries(["Text", "View"].map((n) => [n, n])),
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
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsPage: "SettingsPage",
      SettingsRow: "SettingsRow",
    },
    "../../components/ChallengeCard": {
      ApprovalDecisionPair: "ApprovalDecisionPair",
    },
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/FullScreenLoading": {
      FullScreenLoading: "FullScreenLoading",
    },
    "../../components/ToastOverlay": { ToastOverlay: "ToastOverlay" },
    "./ApprovalEvidence": {
      ApprovalEvidence: "ApprovalEvidence",
      approvalExpiryLabel: () => "Expiry label",
    },
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
    "../../theme/designTokens": { spacing: {}, typeScale: {} },
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
    openedUrls,
    setFrontendUrl: (url: string) => {
      frontendUrl = url;
    },
    gate,
    guard: () => guard,
    pair: () =>
      nodes(h.flush()).find((n) => n.type === "ApprovalDecisionPair")!,
    row: (label: string) =>
      nodes(h.flush()).find(
        (n) => n.type === "SettingsRow" && n.props.label === label,
      )!,
  };
}

test("the native approval detail page preserves grant duration and blocks Back during the real decision promise", async () => {
  const e = setup();
  assert.equal(e.guard().defaultRole, "back");
  assert.equal(e.guard().disabled, false);
  assert.equal(e.pair().props.disabled, false);
  const request = e.pair().props.onApprove();
  e.h.flush();
  assert.deepEqual(e.calls, [["request-1", "APPROVE", 7 * 86400]]);
  assert.equal(e.guard().disabled, true);
  assert.equal(e.pair().props.disabled, true);
  e.gate.resolve();
  await request;
  e.h.flush();
  assert.equal(e.guard().disabled, false);
  e.h.unmount();
});

test("denial on the native detail page sends no reusable grant duration", async () => {
  const e = setup();
  const request = e.pair().props.onDeny();
  assert.deepEqual(e.calls, [["request-1", "DENY", undefined]]);
  e.gate.resolve();
  await request;
  e.h.unmount();
});

test("the approval detail reads as label/value groups ahead of the decision pair", () => {
  const e = setup();
  assert.equal(e.row("Action").props.value, "Write");
  assert.equal(e.row("Resource").props.value, "repo");
  assert.equal(e.row("Requester").props.value, "reviewer");
  assert.equal(e.row("Status").props.value, "Pending");
  assert.equal(e.row("Risk level").props.value, "Low");
  assert.equal(e.row("Grant duration").props.value, "7 days");
  assert.equal(e.row("Expires").props.value, "Expiry label");
  assert.equal(e.row("GitHub").props.detail, "Review write");
  e.h.unmount();
});

test("execution proposal detail retains the redesigned groups and has only a web-review action", () => {
  const e = setup({
    version: 1,
    state: "pending",
    review_path: "/assistant/approvals",
  });
  assert.equal(e.row("Action").props.value, "Write");
  assert.equal(e.row("Requester").props.value, "reviewer");
  assert.equal(e.row("Expires").props.value, "Expiry label");
  assert.equal(e.pair(), undefined);
  const link = e.row("Review in the web app");
  assert.equal(link.props.tone, "action");
  assert.equal(link.props.accessibilityRole, "link");
  link.props.onPress();
  assert.deepEqual(e.openedUrls, ["https://app.example.test/assistant/approvals"]);
  e.setFrontendUrl("https://selected.example.test");
  link.props.onPress();
  assert.deepEqual(e.openedUrls, [
    "https://app.example.test/assistant/approvals",
    "https://selected.example.test/assistant/approvals",
  ]);
  assert.deepEqual(e.calls, []);
  e.h.unmount();
});
