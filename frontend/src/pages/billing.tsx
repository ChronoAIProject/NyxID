import { useEffect } from "react";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { periods } from "@/lib/billing-display";
import { groupRows } from "@/lib/billing-usage";
import {
  normalizeBillingSearch,
  type BillingSearch,
  type BillingUsagePeriod,
} from "@/schemas/billing";
import "@/components/billing/billing-page.css";

export function BillingPage() {
  const search = normalizeBillingSearch(useSearch({ strict: false }));
  const navigate = useNavigate();
  const { tab, period } = search;
  const walletQuery = useBillingWallet();
  const usageQuery = useBillingUsage(period);
  const catalogQuery = useCatalog({ includeAll: true });
  const provisionWallet = useProvisionBillingWallet();
  const topUpBilling = useTopUpBilling();
  const catalog = catalogQuery.data ?? [];
  const services = groupRows(catalog, usageQuery.data?.rows ?? [], "service");
  const service = services.some((group) => group.key === search.service)
    ? search.service
    : "all";
  const rows =
    service === "all"
      ? (usageQuery.data?.rows ?? [])
      : services.find((group) => group.key === service)!.rows;
  const walletUnavailable = isBillingNotConfigured(walletQuery.error);
  const wallet = walletQuery.data;
  const billingCapability = usageQuery.data?.billing;
  const billingReady = Boolean(
    billingCapability?.charging_enabled && billingCapability?.lago_configured,
  );

  function updateSearch(patch: Partial<BillingSearch>) {
    void navigate({ to: "/billing", search: { ...search, ...patch } });
  }
  useEffect(() => {
    if (
      usageQuery.isSuccess &&
      !usageQuery.isFetching &&
      service !== search.service
    ) {
      void navigate({
        to: "/billing",
        search: { ...search, service: "all" },
        replace: true,
      });
    }
  }, [usageQuery.isSuccess, usageQuery.isFetching, service, search, navigate]);
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
          />
          <BillingBenefits catalog={catalog} />
          <BillingTopUpHistory />
        </TabsContent>
        <TabsContent value="usage" className="mt-6 space-y-6">
          <div className="usage-filters">
            <label>
              <span>Service</span>
              <Select
                value={service}
                onValueChange={(value) => updateSearch({ service: value })}
                disabled={usageQuery.isLoading || usageQuery.isError}
              >
                <SelectTrigger aria-label="Service filter">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All active services</SelectItem>
                  {services.map((group) => (
                    <SelectItem key={group.key} value={group.key}>
                      {group.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <label>
              <span>Time</span>
              <Select
                value={period}
                onValueChange={(value) =>
                  updateSearch({ period: value as BillingUsagePeriod })
                }
              >
                <SelectTrigger aria-label="Time filter">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {Object.entries(periods).map(([value, label]) => (
                    <SelectItem key={value} value={value}>
                      {label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <p>Services with recorded usage in this period.</p>
          </div>
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
                      service === "all" ? usageQuery.data?.totals : undefined
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

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function isBillingNotConfigured(error: unknown): boolean {
  return error instanceof ApiError && error.errorCode === 11301;
}
