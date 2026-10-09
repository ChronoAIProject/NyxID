import { useAccountPanel } from "@/hooks/use-account-panel";
import { billingPanelSearch, type AccountPanelParams } from "@/lib/assistant/account-panel-search";
import { useEffect, useEffectEvent, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import { openExternal } from "@/lib/navigation";
import {
  useBillingUsage,
  useBillingWallet,
  useProvisionBillingWallet,
  useTopUpBilling,
} from "@/hooks/use-billing";
import { useCatalog } from "@/hooks/use-keys";
import { BillingBenefits } from "@/components/billing/billing-benefits";
import { BillingWalletCard } from "@/components/billing/billing-wallet-card";
import { BillingTopUpHistory } from "@/components/billing/billing-topup-history";
import { BillingUsageSummary } from "@/components/billing/billing-usage-summary";
import { BillingUsageExplorer } from "@/components/billing/billing-usage-explorer";
import { BillingActivity } from "@/components/billing/billing-activity";
import { BenefitHelp } from "@/components/billing/benefit-help";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { periods } from "@/lib/billing-display";
import { groupRows } from "@/lib/billing-usage";
import { AnalyticsSelect } from "@/components/billing-analytics/filter-picker";
import { BillingMultiSelect } from "@/components/billing/billing-multi-select";
import {
  BILLING_SERVICE_FILTER_LIMIT,
  normalizeBillingSearch,
  type BillingSearch,
  type BillingUsagePeriod,
  type BillingUsageRow,
} from "@/schemas/billing";
import "@/components/billing/billing-page.css";

export function BillingPage({
  presentation = "page",
}: {
  readonly presentation?: "page" | "panel";
} = {}) {
  const account = useAccountPanel();
  const locationSearch: Record<string, unknown> = useSearch({ strict: false });
  const rawSearch = presentation === "panel" ? billingPanelSearch(account.current) : locationSearch;
  const search = normalizeBillingSearch(rawSearch);
  const navigate = useNavigate();
  const { tab, period } = search;
  const [topUpOpen, setTopUpOpen] = useState(false);
  const walletQuery = useBillingWallet();
  const usageQuery = useBillingUsage(period);
  const catalogQuery = useCatalog({ includeAll: true });
  const provisionWallet = useProvisionBillingWallet();
  const topUpBilling = useTopUpBilling();
  const catalog = catalogQuery.data ?? [];
  const services = groupRows(catalog, usageQuery.data?.rows ?? [], "service");
  const selected = search.services.filter((key) =>
    services.some((group) => group.key === key),
  );
  const rows = selected.length
    ? services
        .filter((group) => selected.includes(group.key))
        .flatMap((group) => group.rows)
    : (usageQuery.data?.rows ?? []);
  const walletUnavailable = isBillingNotConfigured(walletQuery.error);
  const wallet = walletQuery.data;
  const billingCapability = usageQuery.data?.billing;
  const billingReady = Boolean(
    billingCapability?.charging_enabled && billingCapability?.lago_configured,
  );

  // Automatic replace cleanup preserves `action`, so filter cleanup that
  // settles before the wallet cannot swallow the deep link. A user-initiated
  // (pushed) navigation abandons a pending top-up: it is consumed in the
  // current entry first and never copied forward, so Back cannot replay it.
  function updateSearch(patch: Partial<BillingSearch>, replace = false) {
    if (presentation === "panel") {
      const params: AccountPanelParams = {
        ...("tab" in patch ? { panelTab: patch.tab } : {}),
        ...("period" in patch ? { panelPeriod: patch.period } : {}),
        ...("services" in patch ? { panelServices: patch.services?.length ? patch.services : undefined, panelService: undefined } : {}),
        ...("action" in patch ? { panelAction: patch.action } : {}),
      };
      void account.update(params, replace ? "replace" : "push");
      return;
    }
    if (!replace && search.action) {
      void navigate({
        to: "/billing",
        search: billingUrlSearch({ ...search, action: undefined }),
        replace: true,
      });
    }
    void navigate({
      to: "/billing",
      search: billingUrlSearch({
        ...search,
        ...(replace ? {} : { action: undefined }),
        ...patch,
      }),
      replace,
    });
  }
  // Prune selections without usage only once the authoritative usage query
  // for this period has settled; this also migrates the legacy `service`.
  const selectionStale =
    selected.length !== search.services.length ||
    rawSearch.service !== undefined;
  const pruneSelection = useEffectEvent(() => updateSearch({ services: selected }, true));
  const selectionKey = JSON.stringify(selected);
  useEffect(() => {
    if (usageQuery.isSuccess && !usageQuery.isFetching && selectionStale) pruneSelection();
  }, [usageQuery.isSuccess, usageQuery.isFetching, selectionStale, selectionKey, search.tab]);
  const topUpReady = Boolean(wallet) && billingReady;
  const topUpSettled =
    !walletQuery.isLoading && (!usageQuery.isLoading || !wallet);
  // `action=topup` opens Add credits once the wallet can take a top-up (else
  // it just lands on the Billing tab), then leaves the URL so refresh or Back
  // never reopens it.
  const wantsTopUp = search.action === "topup" && topUpSettled;
  const [topUpHandled, setTopUpHandled] = useState(false);
  if (wantsTopUp && !topUpHandled) {
    setTopUpHandled(true);
    if (topUpReady) setTopUpOpen(true);
  } else if (!search.action && topUpHandled) {
    setTopUpHandled(false);
  }
  const consumeTopUp = useEffectEvent(() => updateSearch({ tab: "billing", action: undefined }, true));
  useEffect(() => {
    if (wantsTopUp) consumeTopUp();
  }, [wantsTopUp]);
  async function handleProvisionWallet() {
    try {
      await provisionWallet.mutateAsync({});
      toast.success("Billing wallet provisioned");
    } catch (error) {
      toast.error(errorMessage(error, "Failed to provision billing wallet"));
    }
  }

  async function handleTopUp(amountCredits: number) {
    try {
      const checkout = await topUpBilling.mutateAsync({
        amount_credits: amountCredits,
        idempotency_key: crypto.randomUUID(),
      });
      openExternal(checkout.checkout_url);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to create top-up checkout"));
    }
  }

  return (
    <div className="billing-page space-y-6">
      {presentation === "page" && <PageHeader
        title="Billing & Usage"
        description="Your balance, benefits, and usage in one place."
      />}
      <Tabs
        value={tab}
        onValueChange={(value) =>
          updateSearch({ tab: value === "usage" ? "usage" : "billing" })
        }
        className="tabbed-billing"
      >
        <TabsList aria-label="Billing and usage">
          <TabsTrigger value="billing">Billing</TabsTrigger>
          <TabsTrigger value="usage">Usage</TabsTrigger>
        </TabsList>
        {billingCapability && !billingReady && (
          <div className="mt-6 rounded-lg border border-warning/20 bg-warning/5 px-4 py-3 text-12 text-warning">
            Billing is not available on this deployment.
          </div>
        )}
        {catalogQuery.isError && (
          <div className="mt-6">
            <ErrorBanner
              message="Service display names could not be loaded."
              onRetry={() => void catalogQuery.refetch()}
            />
          </div>
        )}
        <TabsContent value="billing" className="mt-6 space-y-6">
          {usageQuery.isError && (
            <ErrorBanner
              message="Billing availability could not be verified. Retry to enable wallet actions."
              onRetry={() => void usageQuery.refetch()}
            />
          )}
          <BillingWalletCard
            titleHelp={
              <BenefitHelp label="Wallet">
                <p>
                  Credits available for paid usage after free allowances and
                  eligible grants. The available balance excludes reservations,
                  unsettled charges, and expiry holds. 1 credit = 1 USD.
                </p>
              </BenefitHelp>
            }
            wallet={wallet}
            loading={walletQuery.isLoading}
            unavailable={walletUnavailable}
            error={walletQuery.error}
            onRetry={() => void walletQuery.refetch()}
            onProvision={() => void handleProvisionWallet()}
            provisioning={provisionWallet.isPending}
            billingReady={billingReady}
            onTopUp={handleTopUp}
            topUpPending={topUpBilling.isPending}
            topUpOpen={topUpOpen}
            onTopUpOpenChange={setTopUpOpen}
          />
          <BillingBenefits catalog={catalog} />
          <BillingTopUpHistory />
        </TabsContent>
        <TabsContent value="usage" className="mt-6 space-y-6">
          <UsageFilters
            services={services}
            values={selected}
            disabled={usageQuery.isLoading || usageQuery.isError}
            onServicesChange={(values) => updateSearch({ services: values })}
            period={period}
            onPeriodChange={(value) => updateSearch({ period: value })}
          />
          {usageQuery.isError ? (
            <ErrorBanner
              message={errorMessage(
                usageQuery.error,
                "Failed to load billing usage.",
              )}
              onRetry={() => void usageQuery.refetch()}
            />
          ) : usageQuery.isLoading ? (
            <Skeleton className="h-[320px] w-full" />
          ) : (
            <>
              <BillingUsageSummary rows={rows} />
              <BillingActivity />
              <BillingUsageExplorer
                catalog={catalog}
                rows={rows}
                totals={selected.length ? undefined : usageQuery.data?.totals}
              />
            </>
          )}
        </TabsContent>
      </Tabs>
    </div>
  );
}

function billingUrlSearch(search: BillingSearch) {
  return {
    tab: search.tab,
    period: search.period,
    services: search.services.length ? search.services : undefined,
    action: search.action,
  };
}

function UsageFilters({
  services,
  values,
  disabled,
  onServicesChange,
  period,
  onPeriodChange,
}: {
  services: { key: string; name: string; rows: BillingUsageRow[] }[];
  values: string[];
  disabled: boolean;
  onServicesChange: (values: string[]) => void;
  period: BillingUsagePeriod;
  onPeriodChange: (period: BillingUsagePeriod) => void;
}) {
  return (
    <BillingMultiSelect
      label="Services"
      itemLabel="Service"
      emptyLabel="All"
      description="Services with recorded usage in this period."
      className="rounded-xl border border-border/50 bg-card px-4 py-3"
      disabled={disabled}
      values={values}
      onChange={onServicesChange}
      options={services.map((group) => ({
        id: group.key,
        label: group.name,
        detail: group.rows[0]?.service_slug ?? undefined,
      }))}
      limit={BILLING_SERVICE_FILTER_LIMIT}
    >
      <div className="ml-auto min-w-0">
        <AnalyticsSelect
          label="Time range"
          inline
          value={period}
          onChange={(value) => onPeriodChange(value as BillingUsagePeriod)}
          options={Object.entries(periods).map(([value, label]) => ({
            value,
            label,
          }))}
        />
      </div>
    </BillingMultiSelect>
  );
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function isBillingNotConfigured(error: unknown): boolean {
  return error instanceof ApiError && error.errorCode === 11301;
}
