import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { useQuery, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { z } from "zod";
import { ActionButton, Label, Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { SettingsGroup, SettingsPage, SettingsRow } from "../../components/settings/SettingsList";
import { SegmentControl } from "../../components/SegmentControl";
import { SkeletonRows } from "../../components/Skeleton";
import { usePullRefresh } from "../../hooks/usePullRefresh";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { billingApi } from "../../lib/api/billingApi";
import { ApiError } from "../../lib/api/ApiError";
import { useAuthSession } from "../auth/AuthSessionContext";
import { billingPeriodSchema, serviceUsage, type BillingPeriod } from "./billingContracts";
import type { BillingRead } from "./billingTransport";
import { BillingHistoryRow, BillingServiceCard, BillingWalletCard } from "./BillingPresentation";

// ownerId is retained for route compatibility; this screen reads personal billing only.
export type UsageCreditsRouteParams = { period?: BillingPeriod; ownerId?: string;
};
export type UsageCreditsScreenProps = { route?: { params?: UsageCreditsRouteParams };
};
const periods = ["24h", "7d", "30d"] as const;

function message(error: unknown) {
  if (error instanceof z.ZodError) return translateApp(
      "This server returned unsupported billing data. Please try again later.",
    );
  if (error instanceof ApiError && error.statusCode === 403) return translateApp("You do not have access to these billing details.");
  return translateApp("Could not load billing details. Please try again.");
}

type ReadQuery = Pick<UseQueryResult<BillingRead<unknown>>, "data" | "isError" | "error">;
function unavailable(queries: ReadQuery[]) {
  const failed = queries.find((query) => query.isError);
  if (failed) return message(failed.error);
  if (queries.some((query) => query.data?.kind === "unsupported")) return translateApp("Billing isn't available on this server.");
  if (queries.some((query) => query.data?.kind === "disabled")) return translateApp("Billing isn't enabled for this account.");
  return undefined;
}

export function UsageCreditsScreen({ route }: UsageCreditsScreenProps = {}) {
  useAppLanguage();
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
    if (!isConnected && !(await recheckConnection())) return;
    if (!canRefresh.current || getAuthEpoch() !== epoch) return;
    await Promise.allSettled([wallet.refetch(), grants.refetch(), allowances.refetch(), ...(historyOpen.current && hasWallet && !issue ? [usage.refetch()] : [])]);
  }
  // `fetching` still gates Retry; only a pull shows the spinner.
  const pull = usePullRefresh(refresh);
  function toggleHistory() {
    if (expanded) void client.cancelQueries({ queryKey: [...root, "usage"] });
    historyOpen.current = !expanded;
    setHistory({ epoch, expanded: !expanded });
  }
  if (!isAuthenticated) return (
      <WorkspacePage title={translateApp("Usage & credits")}
        showHeading={false}><Label>
          {translateApp("Sign in to review your usage and credits.")}
        </Label></WorkspacePage>
    );
  const loaded = wallet.data?.kind === "ready" && wallet.data.data && grants.data?.kind === "ready" && allowances.data?.kind === "ready"
    ? { wallet: wallet.data.data, grants: grants.data.data.grants, allowances: allowances.data.data.allowances } : undefined;
  const historyIssue = expanded ? unavailable([usage]) : undefined;
  return (
    <SettingsPage refreshing={pull.refreshing} onRefresh={pull.onRefresh}>
    {issue ? (
        <>
      <Notice tone="error" message={issue} />
      <ActionButton label={translateApp("Retry")}
            variant="secondary" disabled={fetching} onPress={() => void refresh()} />
    </>
      ) : absent ? (
        <Label muted>{translateApp("No billing account yet.")}</Label>
      ) : !loaded ? (
        <>
      {isConnected ? (
            <SkeletonRows rows={3} icon={false} detail={false} title accessibilityLabel={translateApp("Loading balance")}
            />
          ) : (
            <Label muted>
              {translateApp("Offline. Connect to load your balance.")}
            </Label>
          )}
      {!isConnected && (
            <ActionButton label={translateApp("Retry")}
              variant="secondary" onPress={() => void refresh()} />
          )}
    </>
      ) : (
        <>
      {!isConnected && (
            <Notice message={translateApp("Offline. Showing your saved balance.")}
            />
          )}
      <BillingWalletCard {...loaded} now={Date.now()} />
      <BillingHistoryRow expanded={expanded} onPress={toggleHistory} />
      {expanded && (
            <>
        <SegmentControl segments={[{ label: translateApp("24 hours") }, { label: translateApp("7 days") }, { label: translateApp("30 days") }]} activeIndex={periods.indexOf(period)} onPress={(index) => setPeriod(periods[index] ?? "30d")} />
        {historyIssue ? (
                <>
          <Notice tone="error" message={historyIssue} />
          <ActionButton label={translateApp("Retry")}
                    variant="secondary" disabled={fetching} onPress={() => void refresh()} />
        </>
              ) : usage.data?.kind === "ready" ? (
                <SettingsGroup title={translateApp("Usage by service")}
                  inset={false}
          footer={usage.data.data.billing.rates_are_approximate ? translateApp("Usage costs are estimates.")
                      : undefined}>
          {services.length === 0 && (
                    <SettingsRow label={translateApp("No usage recorded for this period")}
                    />
                  )}
          {services.slice(0, serviceLimit).map((service) => (
                    <BillingServiceCard key={`${period}:${service.key}`} service={service} />))}
          {services.length > serviceLimit && (
                    <SettingsRow tone="action" label={translateApp("Show More Services")}
                      onPress={() => setServiceLimit((limit) => limit + 10)} />
                  )}
        </SettingsGroup>
              ) : isConnected ? (
                <SkeletonRows rows={3} icon={false} title accessibilityLabel={translateApp("Loading usage")}
                />
              ) : (
                <Label muted>
                  {translateApp("Offline. Connect to load usage.")}
                </Label>
              )}
      </>
          )}
    </>
      )}
  </SettingsPage>
  );
}
