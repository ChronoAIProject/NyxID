import { useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { useQuery, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { z } from "zod";
import { ActionButton, Label, Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { SegmentControl } from "../../components/SegmentControl";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { billingApi } from "../../lib/api/billingApi";
import { ApiError } from "../../lib/api/ApiError";
import { useAuthSession } from "../auth/AuthSessionContext";
import { billingPeriodSchema, serviceUsage, type BillingPeriod } from "./billingContracts";
import type { BillingRead } from "./billingTransport";
import { BillingHistoryRow, BillingServiceCard, BillingWalletCard } from "./BillingPresentation";

// ownerId is retained for route compatibility; this screen reads personal billing only.
export type UsageCreditsRouteParams = { period?: BillingPeriod; ownerId?: string };
export type UsageCreditsScreenProps = { route?: { params?: UsageCreditsRouteParams } };
const periods = ["24h", "7d", "30d"] as const;

function message(error: unknown) {
  if (error instanceof z.ZodError) return "This server returned unsupported billing data. Please try again later.";
  if (error instanceof ApiError && error.statusCode === 403) return "You do not have access to these billing details.";
  return "Could not load billing details. Please try again.";
}

type ReadQuery = Pick<UseQueryResult<BillingRead<unknown>>, "data" | "isError" | "error">;
function unavailable(queries: ReadQuery[]) {
  const failed = queries.find((query) => query.isError);
  if (failed) return message(failed.error);
  if (queries.some((query) => query.data?.kind === "unsupported")) return "Billing isn't available on this server.";
  if (queries.some((query) => query.data?.kind === "disabled")) return "Billing isn't enabled for this account.";
  return undefined;
}

export function UsageCreditsScreen({ route }: UsageCreditsScreenProps = {}) {
  const [period, setPeriod] = useState<BillingPeriod>(() => billingPeriodSchema.catch("30d").parse(route?.params?.period));
  const [history, setHistory] = useState({ epoch: getAuthEpoch(), expanded: false });
  const [foreground, setForeground] = useState(AppState.currentState === "active");
  const [serviceLimit, setServiceLimit] = useState(10);
  const focused = useIsFocused();
  const { isConnected, recheckConnection } = useNetworkStatus();
  const { isAuthenticated } = useAuthSession();
  const epoch = getAuthEpoch();
  const expanded = history.epoch === epoch && history.expanded;
  const active = focused && foreground && isConnected && isAuthenticated;
  const canRefresh = useRef(false);
  const historyOpen = useRef(false);
  canRefresh.current = focused && foreground && isAuthenticated;
  historyOpen.current = expanded;
  const client = useQueryClient();
  const root = ["workspace", "billing", epoch] as const;
  useEffect(() => {
    const listener = AppState.addEventListener("change", (state) => setForeground(state === "active"));
    return () => listener.remove();
  }, []);
  useEffect(() => () => { void client.cancelQueries({ queryKey: ["workspace", "billing", epoch] }); }, [active, epoch, client]);
  useEffect(() => () => { canRefresh.current = false; }, []);
  useEffect(() => { setPeriod(billingPeriodSchema.catch("30d").parse(route?.params?.period)); }, [route?.params?.period]);
  const wallet = useQuery({ queryKey: [...root, "wallet"], queryFn: ({ signal }) => billingApi.wallet(signal), enabled: active, retry: false, staleTime: 30_000 });
  const grants = useQuery({ queryKey: [...root, "grants", "personal"], queryFn: ({ signal }) => billingApi.grants(undefined, signal), enabled: active, retry: false, staleTime: 30_000 });
  const allowances = useQuery({ queryKey: [...root, "allowances", "personal"], queryFn: ({ signal }) => billingApi.allowances(undefined, signal), enabled: active, retry: false, staleTime: 30_000 });
  const absent = !wallet.isError && wallet.data?.kind === "ready" && wallet.data.data === null;
  const issue = unavailable(absent ? [wallet] : [wallet, grants, allowances]);
  const hasWallet = wallet.data?.kind === "ready" && wallet.data.data !== null;
  const usage = useQuery({ queryKey: [...root, "usage", period], queryFn: ({ signal }) => billingApi.usage(period, signal), enabled: active && expanded && hasWallet && !issue, retry: false, staleTime: 30_000 });
  const services = useMemo(() => usage.data?.kind === "ready" ? serviceUsage(usage.data.data.rows) : [], [usage.data]);
  useEffect(() => { setServiceLimit(10); }, [period, epoch]);
  const fetching = wallet.isFetching || grants.isFetching || allowances.isFetching || (expanded && usage.isFetching);
  async function refresh() {
    if (fetching || !focused || !foreground || !isAuthenticated) return;
    if (!isConnected && !await recheckConnection()) return;
    if (!canRefresh.current || getAuthEpoch() !== epoch) return;
    await Promise.allSettled([wallet.refetch(), grants.refetch(), allowances.refetch(), ...(historyOpen.current && hasWallet && !issue ? [usage.refetch()] : [])]);
  }
  function toggleHistory() {
    if (expanded) void client.cancelQueries({ queryKey: [...root, "usage"] });
    historyOpen.current = !expanded;
    setHistory({ epoch, expanded: !expanded });
  }
  if (!isAuthenticated) return <WorkspacePage title="Usage & credits" showHeading={false}><Label>Sign in to review your usage and credits.</Label></WorkspacePage>;
  const loaded = wallet.data?.kind === "ready" && wallet.data.data && grants.data?.kind === "ready" && allowances.data?.kind === "ready"
    ? { wallet: wallet.data.data, grants: grants.data.data.grants, allowances: allowances.data.data.allowances } : undefined;
  const historyIssue = expanded ? unavailable([usage]) : undefined;
  return <WorkspacePage title="Usage & credits" showHeading={false} refreshing={fetching} onRefresh={() => void refresh()}>
    {issue ? <>
      <Notice tone="error" message={issue} />
      <ActionButton label="Retry" variant="secondary" disabled={fetching} onPress={() => void refresh()} />
    </> : absent ? <Label muted>No billing account yet.</Label> : !loaded ? <>
      <Label muted>{isConnected ? "Loading…" : "Offline. Connect to load your balance."}</Label>
      {!isConnected && <ActionButton label="Retry" variant="secondary" onPress={() => void refresh()} />}
    </> : <>
      {!isConnected && <Notice message="Offline. Showing your saved balance." />}
      <BillingWalletCard {...loaded} now={Date.now()} />
      <Label muted style={{ fontSize: 12, lineHeight: 16 }}>To add credits, use NyxID on the web.</Label>
      <BillingHistoryRow expanded={expanded} onPress={toggleHistory} />
      {expanded && <>
        <SegmentControl segments={[{ label: "24 hours" }, { label: "7 days" }, { label: "30 days" }]} activeIndex={periods.indexOf(period)} onPress={(index) => setPeriod(periods[index] ?? "30d")} />
        {historyIssue ? <>
          <Notice tone="error" message={historyIssue} />
          <ActionButton label="Retry" variant="secondary" disabled={fetching} onPress={() => void refresh()} />
        </> : usage.data?.kind === "ready" ? <>
          {usage.data.data.billing.rates_are_approximate && <Label muted>Usage costs are estimates.</Label>}
          {services.length === 0 && <Label muted>No usage recorded for this period.</Label>}
          {services.slice(0, serviceLimit).map((service) => <BillingServiceCard key={`${period}:${service.key}`} service={service} />)}
          {services.length > serviceLimit && <ActionButton label="Show more services" variant="secondary" onPress={() => setServiceLimit((limit) => limit + 10)} />}
        </> : <Label muted>{isConnected ? "Loading usage…" : "Offline. Connect to load usage."}</Label>}
      </>}
    </>}
  </WorkspacePage>;
}
