import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { ApiError } from "../../lib/api/ApiError";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import * as contracts from "./billingContracts";
import * as credits from "./credits";
import { allowanceFixture, grantFixture, ORG_ADMIN, usageFixture, walletFixture } from "./billingFixtures";

type Element = { type: unknown; props: Record<string, any> };
type Query = { queryKey: unknown[]; enabled: boolean; queryFn: (context: { signal: AbortSignal }) => Promise<unknown> };
type State = { data?: unknown; isPending?: boolean; isError?: boolean; isFetching?: boolean; error?: unknown };
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));
function elements(root: Element): Element[] {
  return [root, ...[root.props.children].flat(Infinity).flatMap((child) => child && typeof child === "object" && "props" in child ? elements(child) : [])];
}
const text = (root: Element) => elements(root).flatMap((item) => [item.props.children, item.props.message].flat(Infinity)).filter((value) => typeof value === "string").join(" ");
const ready = (data: unknown) => ({ data: { kind: "ready", data } });

function setup({ params = {}, states = {} }: { params?: { period?: string; ownerId?: string }; states?: Record<string, State> } = {}) {
  let online = true;
  let authenticated = true;
  let focused = true;
  let epoch = 4;
  let recheck = async () => true;
  const reads: Array<{ method: string; args: unknown[] }> = [];
  const refreshed: string[] = [];
  const cancelled: unknown[][] = [];
  const queries = new Map<string, Query>();
  const listeners = new Set<(state: string) => void>();
  const appState = { currentState: "active", addEventListener: (_event: string, listener: (state: string) => void) => {
    listeners.add(listener);
    return { remove: () => { listeners.delete(listener); } };
  } };
  const queryClient = { cancelQueries: async ({ queryKey }: { queryKey: unknown[] }) => { cancelled.push(plain(queryKey)); } };
  const defaults: Record<string, State> = {
    wallet: ready(contracts.walletSchema.parse(walletFixture)),
    usage: ready(contracts.usageSchema.parse(usageFixture)),
    grants: ready(contracts.grantsSchema.parse({ grants: [grantFixture] })),
    allowances: ready(contracts.allowancesSchema.parse({ allowances: [allowanceFixture] })),
  };
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": { AppState: appState, View: "View", StyleSheet: { create: (styles: unknown) => styles } },
    "@react-navigation/native": { useIsFocused: () => focused },
    "@tanstack/react-query": {
      useQueryClient: () => queryClient,
      useQuery: (query: Query) => {
        const kind = String(query.queryKey[3]);
        queries.set(kind, query);
        const state = states[kind] ?? defaults[kind]!;
        return { isPending: false, isFetching: false, isError: false, ...state, refetch: async () => { refreshed.push(kind); } };
      },
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(["ActionButton", "Card", "Label", "Notice", "WorkspacePage"].map((name) => [name, name])),
    "../../components/SegmentControl": { SegmentControl: "SegmentControl" },
    "../../hooks/useNetworkStatus": { useNetworkStatus: () => ({ isConnected: online, recheckConnection: () => recheck() }) },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../auth/AuthSessionContext": { useAuthSession: () => ({ isAuthenticated: authenticated }) },
    "../../lib/api/ApiError": { ApiError },
    "../../lib/api/billingApi": { billingApi: Object.fromEntries(["wallet", "organizations", "usage", "grants", "allowances"].map((method) => [method, async (...args: unknown[]) => { reads.push({ method, args }); }])) },
    "./billingContracts": contracts,
    "./credits": credits,
    "./BillingPresentation": Object.fromEntries(["BillingHistoryRow", "BillingServiceCard", "BillingWalletCard"].map((name) => [name, name])),
  });
  const { UsageCreditsScreen } = harness.load(fileURLToPath(new URL("./UsageCreditsScreen.tsx", import.meta.url))) as { UsageCreditsScreen: (props: unknown) => Element };
  harness.mount(() => UsageCreditsScreen({ route: { params } }));
  const all = () => elements(harness.value());
  const find = (type: string, label?: string) => {
    const item = all().find((item) => item.type === type && (label === undefined || item.props.label === label));
    assert.ok(item, `Expected ${type} ${label ?? ""}`);
    return item;
  };
  return {
    harness, queries, reads, refreshed, cancelled, all, find,
    text: () => text(harness.value()),
    states,
    toggle: () => { find("BillingHistoryRow").props.onPress(); harness.flush(); },
    refresh: async () => { harness.value().props.onRefresh(); await harness.settle(); },
    setOnline: (value: boolean) => { online = value; harness.render(); },
    setFocused: (value: boolean) => { focused = value; harness.render(); },
    setAuthenticated: (value: boolean) => { authenticated = value; harness.render(); },
    setEpoch: (value: number) => { epoch = value; harness.render(); },
    setRecheck: (value: () => Promise<boolean>) => { recheck = value; },
    appState: (value: string) => { appState.currentState = value; for (const listener of listeners) listener(value); harness.flush(); },
    readEnabled: async () => {
      const signal = new AbortController().signal;
      await Promise.all([...queries.values()].filter((query) => query.enabled).map((query) => query.queryFn({ signal })));
    },
  };
}

test("personal balance alone is visible, without headings, account selectors or eager usage", async (t) => {
  const screen = setup({ params: { ownerId: ORG_ADMIN } });
  t.after(screen.harness.unmount);
  assert.equal(screen.harness.value().props.showHeading, false);
  assert.equal(screen.harness.value().props.subtitle, undefined);
  assert.equal(screen.find("BillingWalletCard").props.wallet.balance, "-0.000000000001");
  assert.equal(screen.find("BillingWalletCard").props.grants.length, 1);
  assert.equal(screen.find("BillingWalletCard").props.allowances.length, 1);
  assert.equal(screen.find("BillingHistoryRow").props.expanded, false);
  assert.match(screen.text(), /To add credits, use NyxID on the web\./);
  assert.doesNotMatch(screen.text(), /Personal usage|Credit grants|account selection|Personal account/);
  assert.equal(screen.all().some((item) => item.type === "SegmentControl" || item.type === "BillingServiceCard"), false);
  assert.equal(screen.all().some((item) => item.props.accessibilityRole === "link" || item.props.href || item.props.url), false);
  assert.equal(screen.all().some((item) => item.type === "ActionButton" && /top.?up|buy|purchase|checkout|add credits/i.test(item.props.label)), false);
  await screen.readEnabled();
  assert.deepEqual(screen.reads.map((read) => read.method).sort(), ["allowances", "grants", "wallet"]);
  for (const method of ["grants", "allowances"]) assert.equal(screen.reads.find((read) => read.method === method)!.args[0], undefined);
  await screen.refresh();
  assert.deepEqual(screen.refreshed.sort(), ["allowances", "grants", "wallet"]);
});

test("history expansion enables usage and period selection; collapse cancels and excludes it from refresh", async (t) => {
  const screen = setup({ params: { period: "7d" } });
  t.after(screen.harness.unmount);
  assert.equal(screen.queries.get("usage")!.enabled, false);
  screen.toggle();
  assert.equal(screen.queries.get("usage")!.enabled, true);
  assert.deepEqual(plain(screen.queries.get("usage")!.queryKey), ["workspace", "billing", 4, "usage", "7d"]);
  assert.equal(screen.find("BillingServiceCard").props.service.cost, "9007199254740993.000000000003");
  screen.find("SegmentControl").props.onPress(0);
  screen.harness.flush();
  await screen.readEnabled();
  assert.equal(screen.reads.find((call) => call.method === "usage")!.args[0], "24h");
  await screen.refresh();
  assert.ok(screen.refreshed.includes("usage"));
  screen.toggle();
  assert.equal(screen.queries.get("usage")!.enabled, false);
  assert.equal(screen.all().some((item) => item.type === "BillingServiceCard"), false);
  assert.ok(screen.cancelled.some((key) => key[3] === "usage"));
  screen.refreshed.length = 0;
  await screen.refresh();
  assert.equal(screen.refreshed.includes("usage"), false);
});

for (const state of ["unsupported", "disabled"] as const) {
  test(`${state} billing renders one whole-screen state`, async (t) => {
    const screen = setup({ states: { wallet: { data: { kind: state } }, grants: { data: { kind: state } }, allowances: { data: { kind: state } } } });
    t.after(screen.harness.unmount);
    assert.equal(screen.all().filter((item) => item.type === "Notice").length, 1);
    assert.match(screen.text(), state === "unsupported" ? /Billing isn't available on this server/ : /Billing isn't enabled for this account/);
    assert.equal(screen.all().some((item) => item.type === "BillingWalletCard" || item.type === "BillingHistoryRow"), false);
    await screen.find("ActionButton", "Retry").props.onPress();
    await screen.harness.settle();
    assert.deepEqual(screen.refreshed.sort(), ["allowances", "grants", "wallet"]);
    screen.states.wallet = ready(null);
    screen.states.grants = ready({ grants: [] });
    screen.states.allowances = ready({ allowances: [] });
    screen.harness.render();
    assert.match(screen.text(), /No billing account yet/);
    assert.doesNotMatch(screen.text(), /isn't available|isn't enabled/);
    assert.equal(screen.queries.get("usage")!.enabled, false);
  });
}

test("loading, offline without cache and empty balances do not show section placeholders", async (t) => {
  const screen = setup({ states: { wallet: { isPending: true } } });
  t.after(screen.harness.unmount);
  assert.match(screen.text(), /Loading…/);
  screen.setOnline(false);
  assert.match(screen.text(), /Offline. Connect to load your balance/);
  await screen.readEnabled();
  assert.equal(screen.reads.length, 0);
  screen.states.wallet = ready(contracts.walletSchema.parse(walletFixture));
  screen.states.grants = ready({ grants: [] });
  screen.states.allowances = ready({ allowances: [] });
  screen.harness.render();
  assert.match(screen.text(), /Offline. Showing your saved balance/);
  assert.equal(screen.all().some((item) => item.type === "BillingWalletCard"), true);
  assert.doesNotMatch(screen.text(), /No active credit grants|No usage allowances/);
});

test("explicit wallet absence remains distinct even when benefit routes are unsupported", (t) => {
  const screen = setup({ states: { wallet: ready(null), grants: { data: { kind: "unsupported" } }, allowances: { data: { kind: "unsupported" } } } });
  t.after(screen.harness.unmount);
  assert.match(screen.text(), /No billing account yet/);
  assert.doesNotMatch(screen.text(), /isn't available/);
});

test("collapsed history ignores cached errors; expanded history has one retry state or an empty list", async (t) => {
  const screen = setup({ states: { usage: { ...ready(contracts.usageSchema.parse(usageFixture)), isError: true, error: new Error("network") } } });
  t.after(screen.harness.unmount);
  assert.doesNotMatch(screen.text(), /Could not load/);
  screen.toggle();
  assert.match(screen.text(), /Could not load billing details/);
  assert.equal(screen.all().some((item) => item.type === "BillingServiceCard"), false);
  assert.equal(screen.all().filter((item) => item.type === "Notice").length, 1);
  screen.states.usage = ready(contracts.usageSchema.parse({ ...usageFixture, rows: [], billing: { charging_enabled: false, rates_are_approximate: true } }));
  screen.harness.render();
  assert.match(screen.text(), /No usage recorded for this period/);
  screen.toggle();
  assert.doesNotMatch(screen.text(), /No usage recorded/);
});

test("ACL errors hide stale balances and offer one concise retry", (t) => {
  const error = new ApiError({ statusCode: 403, errorKey: "forbidden", errorCode: 1003, message: "Denied" });
  const screen = setup({ states: { grants: { ...ready(contracts.grantsSchema.parse({ grants: [grantFixture] })), isError: true, error } } });
  t.after(screen.harness.unmount);
  assert.match(screen.text(), /do not have access/);
  assert.equal(screen.all().some((item) => item.type === "BillingWalletCard"), false);
  assert.equal(screen.all().filter((item) => item.type === "Notice").length, 1);
});

test("blur, background and signout disable queries; auth epochs separate cache keys", (t) => {
  const screen = setup();
  t.after(screen.harness.unmount);
  screen.toggle();
  for (const transition of [() => screen.setFocused(false), () => screen.appState("background"), () => screen.setAuthenticated(false)]) {
    screen.setFocused(true);
    screen.appState("active");
    screen.setAuthenticated(true);
    transition();
    assert.equal([...screen.queries.values()].some((query) => query.enabled), false);
  }
  assert.match(screen.text(), /Sign in to review/);
  assert.equal(screen.all().some((item) => item.type === "BillingWalletCard"), false);
  screen.setEpoch(5);
  screen.setAuthenticated(true);
  assert.equal([...screen.queries.values()].every((query) => query.queryKey[2] === 5), true);
  assert.equal(screen.find("BillingHistoryRow").props.expanded, false);
  assert.equal(screen.queries.get("usage")!.enabled, false);
  assert.ok(screen.cancelled.some((key) => key[2] === 4));
});

for (const transition of ["background", "blur", "signout", "unmount"] as const) {
  test(`offline recheck finishing after ${transition} does not start billing reads`, async (t) => {
    const screen = setup();
    t.after(screen.harness.unmount);
    screen.setOnline(false);
    const pending = deferred<boolean>();
    screen.setRecheck(() => pending.promise);
    screen.harness.value().props.onRefresh();
    if (transition === "background") screen.appState("background");
    if (transition === "blur") screen.setFocused(false);
    if (transition === "signout") { screen.setEpoch(5); screen.setAuthenticated(false); }
    if (transition === "unmount") screen.harness.unmount();
    pending.resolve(true);
    await screen.harness.settle();
    assert.deepEqual(screen.refreshed, []);
  });
}

test("closing history during an offline recheck does not dispatch a usage refresh", async (t) => {
  const screen = setup();
  t.after(screen.harness.unmount);
  screen.toggle();
  screen.setOnline(false);
  const pending = deferred<boolean>();
  screen.setRecheck(() => pending.promise);
  screen.harness.value().props.onRefresh();
  screen.toggle();
  pending.resolve(true);
  await screen.harness.settle();
  assert.equal(screen.refreshed.includes("usage"), false);
});

test("large service lists expand only inside optional history", (t) => {
  const usage = contracts.usageSchema.parse({ ...usageFixture, rows: Array.from({ length: 11 }, (_, i) => ({ ...usageFixture.rows[0], service_id: `service-${i}`, service_slug: `Service ${i}` })) });
  const screen = setup({ states: { usage: ready(usage) } });
  t.after(screen.harness.unmount);
  assert.equal(screen.all().some((item) => item.type === "BillingServiceCard"), false);
  screen.toggle();
  assert.equal(screen.all().filter((item) => item.type === "BillingServiceCard").length, 10);
  screen.find("ActionButton", "Show more services").props.onPress();
  screen.harness.flush();
  assert.equal(screen.all().filter((item) => item.type === "BillingServiceCard").length, 11);
});
