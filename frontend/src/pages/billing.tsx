import { useEffect, useState } from "react";
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
import {
  ServiceUsage,
  UsageMetricsDisclosure,
} from "@/components/billing/billing-usage-details";
import { BenefitHelp } from "@/components/billing/benefit-help";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { periods } from "@/lib/billing-display";
import { groupRows } from "@/lib/billing-usage";
import {
  AnalyticsSelect,
  FilterCard,
  FilterPicker,
} from "@/components/billing-analytics/filter-picker";
import { DataTableFilterChips } from "@/components/data-table/data-table-controls";
import type { DataTableFilterField } from "@/types/data-table";
import {
  BILLING_SERVICE_FILTER_LIMIT,
  normalizeBillingSearch,
  type BillingSearch,
  type BillingUsagePeriod,
  type BillingUsageRow,
} from "@/schemas/billing";
import "@/components/billing/billing-page.css";

export function BillingPage() {
  const rawSearch: Record<string, unknown> = useSearch({ strict: false });
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
  useEffect(() => {
    if (usageQuery.isSuccess && !usageQuery.isFetching && selectionStale) {
      updateSearch({ services: selected }, true);
    }
  });
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
  useEffect(() => {
    if (wantsTopUp) updateSearch({ tab: "billing", action: undefined }, true);
  });
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
      <PageHeader
        title="Billing & Usage"
        description="Your balance, benefits, and usage in one place."
      />
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
          <div className="mt-6 rounded-lg border border-warning/20 bg-warning/5 px-4 py-3 text-[12px] text-warning">
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
              <Card className="usage-results">
                <CardHeader>
                  <CardTitle>Usage breakdown</CardTitle>
                  <p className="text-[12px] text-muted-foreground">
                    Expand a service for its models, agents, and funding.
                  </p>
                </CardHeader>
                <CardContent>
                  <UsageMetricsDisclosure
                    rows={rows}
                    totals={
                      selected.length ? undefined : usageQuery.data?.totals
                    }
                  />
                  <ServiceUsage catalog={catalog} rows={rows} />
                </CardContent>
              </Card>
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

const SERVICE_FILTER_FIELD: DataTableFilterField<"services"> = {
  key: "services",
  label: "Services",
  value_type: "enum",
  operator: "includes",
  multiple: true,
  options: [],
};

/** The admin usage filter card, with the personal usage options and periods. */
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
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const all = services.map((group) => ({
    id: group.key,
    label: group.name,
    detail: group.rows[0]?.service_slug ?? undefined,
  }));
  const options = all.filter((option) =>
    `${option.label} ${option.detail ?? ""}`.toLowerCase().includes(needle),
  );
  const nameOf = (key: string) =>
    all.find((option) => option.id === key)?.label ?? key;
  return (
    <FilterCard
      pickers={
        <FilterPicker
          label="Services"
          description="Services with recorded usage in this period."
          disabled={disabled}
          values={values}
          onChange={onServicesChange}
          open={open}
          onOpenChange={setOpen}
          search={query}
          onSearchChange={setQuery}
          searchPlaceholder="Search services"
          options={{ status: "success", options, total: options.length }}
          limit={BILLING_SERVICE_FILTER_LIMIT}
        />
      }
      aside={
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
      }
    >
      <DataTableFilterChips
        search=""
        searchFields={[]}
        searchFilters={[]}
        filters={
          values.length
            ? [
                {
                  field: SERVICE_FILTER_FIELD,
                  values,
                  valueLabels: values.map(nameOf),
                },
              ]
            : []
        }
        onEditSearch={() => undefined}
        onRemoveSearch={() => undefined}
        onEditSearchValue={() => undefined}
        onRemoveSearchValue={() => undefined}
        onEdit={() => {
          setQuery("");
          setOpen(true);
        }}
        onRemove={() => onServicesChange([])}
        onClear={() => onServicesChange([])}
      />
    </FilterCard>
  );
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function isBillingNotConfigured(error: unknown): boolean {
  return error instanceof ApiError && error.errorCode === 11301;
}
