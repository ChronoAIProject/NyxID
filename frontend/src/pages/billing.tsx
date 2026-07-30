import { useMemo, useState } from "react";
import { toast } from "sonner";
import {
  CreditCard,
  ExternalLink,
  Plus,
  RefreshCw,
  WalletCards,
} from "lucide-react";
import { ApiError } from "@/lib/api-client";
import { openExternal } from "@/lib/navigation";
import {
  BILLING_USAGE_PERIODS,
  type BillingUsagePeriod,
  type BillingUsageRow,
  type BillingWalletResponse,
} from "@/schemas/billing";
import {
  useBillingUsage,
  useBillingWallet,
  useProvisionBillingWallet,
  useTopUpBilling,
  useTopUpHistory,
  openInvoiceReceipt,
} from "@/hooks/use-billing";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const DEFAULT_TOP_UP_CREDITS = 100;

export function BillingPage() {
  const [period, setPeriod] = useState<BillingUsagePeriod>("30d");
  const [topUpCredits, setTopUpCredits] = useState(String(DEFAULT_TOP_UP_CREDITS));
  const walletQuery = useBillingWallet();
  const usageQuery = useBillingUsage(period);
  const provisionWallet = useProvisionBillingWallet();
  const topUpBilling = useTopUpBilling();

  const walletUnavailable = isBillingNotConfigured(walletQuery.error);
  const wallet = walletQuery.data;
  const billingCapability = usageQuery.data?.billing;
  const billingReady =
    Boolean(billingCapability?.charging_enabled) &&
    Boolean(billingCapability?.lago_configured);
  const topUpAmount = Number(topUpCredits);
  const topUpDisabled =
    !billingReady ||
    !Number.isInteger(topUpAmount) ||
    topUpAmount <= 0 ||
    topUpAmount > 10_000_000 ||
    topUpBilling.isPending;

  async function handleProvisionWallet() {
    try {
      await provisionWallet.mutateAsync({});
      toast.success("Billing wallet provisioned");
    } catch (error) {
      toast.error(errorMessage(error, "Failed to provision billing wallet"));
    }
  }

  async function handleTopUp() {
    if (topUpDisabled) return;

    try {
      const checkout = await topUpBilling.mutateAsync({
        amount_credits: topUpAmount,
        idempotency_key: crypto.randomUUID(),
      });
      openExternal(checkout.checkout_url);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to create top-up checkout"));
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h2 className="text-[28px] font-bold leading-none tracking-tight">
            Billing
          </h2>
          <p className="mt-1 text-[12px] text-muted-foreground">
            Wallet balance, credits, and service usage.
          </p>
        </div>
        <Select
          value={period}
          onValueChange={(value) => setPeriod(value as BillingUsagePeriod)}
        >
          <SelectTrigger className="w-full sm:w-[148px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {BILLING_USAGE_PERIODS.map((value) => (
              <SelectItem key={value} value={value}>
                {periodLabel(value)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {usageQuery.isError && (
        <ErrorBanner
          message={errorMessage(usageQuery.error, "Failed to load billing usage.")}
          onRetry={() => void usageQuery.refetch()}
        />
      )}

      {billingCapability && !billingReady && (
        <div className="rounded-lg border border-warning/20 bg-warning/5 px-4 py-3 text-[12px] text-warning">
          Billing is not available on this deployment.
        </div>
      )}

      <div className="grid gap-4 xl:grid-cols-[1.2fr_0.8fr]">
        <WalletCard
          wallet={wallet}
          loading={walletQuery.isLoading}
          unavailable={walletUnavailable}
          error={walletQuery.error}
          onRetry={() => void walletQuery.refetch()}
          onProvision={() => void handleProvisionWallet()}
          provisioning={provisionWallet.isPending}
          billingReady={billingReady}
        />

        <Card>
          <CardHeader className="flex-row items-center justify-between space-y-0">
            <div>
              <CardTitle>Top Up</CardTitle>
              <p className="mt-1 text-[12px] text-muted-foreground">
                Add credits through hosted checkout.
              </p>
            </div>
            <CreditCard className="h-4 w-4 text-text-tertiary" />
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <label className="text-[12px] font-medium" htmlFor="topup-credits">
                Credits
              </label>
              <Input
                id="topup-credits"
                type="number"
                min={1}
                max={10_000_000}
                step={1}
                value={topUpCredits}
                onChange={(event) => setTopUpCredits(event.target.value)}
                disabled={!billingReady || topUpBilling.isPending}
              />
            </div>
            <Button
              variant="primary"
              className="w-full"
              disabled={topUpDisabled}
              isLoading={topUpBilling.isPending}
              onClick={() => void handleTopUp()}
            >
              <ButtonIcon variant="primary">
                <ExternalLink className="h-3 w-3" />
              </ButtonIcon>
              Checkout
            </Button>
          </CardContent>
        </Card>
      </div>

      <UsageSummary
        rows={usageQuery.data?.rows ?? []}
        totals={usageQuery.data?.totals}
        loading={usageQuery.isLoading}
      />

      <TopUpHistory />
    </div>
  );
}

const TOPUP_STATUS_VARIANT = {
  paid: "success",
  pending: "secondary",
  failed: "destructive",
  voided: "secondary",
} as const;

function TopUpHistory() {
  const historyQuery = useTopUpHistory();
  const topups = historyQuery.data?.topups ?? [];

  async function handleReceipt(lagoInvoiceId: string) {
    try {
      await openInvoiceReceipt(lagoInvoiceId);
    } catch (error) {
      toast.error(
        error instanceof ApiError
          ? error.message
          : "The receipt is not ready yet; try again shortly",
      );
    }
  }

  if (historyQuery.isLoading) {
    return <Skeleton className="h-[200px] w-full" />;
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Top-up history</CardTitle>
        <p className="mt-1 text-[12px] text-muted-foreground">
          Payments, their status, and downloadable receipts.
        </p>
      </CardHeader>
      <CardContent>
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Date</TableHead>
                <TableHead className="text-right">Credits</TableHead>
                <TableHead>Invoice</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {topups.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="py-8 text-center text-muted-foreground">
                    No top-ups yet.
                  </TableCell>
                </TableRow>
              ) : (
                topups.map((topup) => (
                  <TableRow key={topup.id}>
                    <TableCell>
                      {new Date(topup.created_at).toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right">
                      {formatNumber(topup.amount_credits)}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {topup.invoice_number ?? "—"}
                    </TableCell>
                    <TableCell>
                      <Badge variant={TOPUP_STATUS_VARIANT[topup.status]}>
                        {labelize(topup.status)}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-right">
                      {topup.status === "pending" && topup.checkout_url ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => openExternal(topup.checkout_url ?? "")}
                        >
                          Resume payment
                        </Button>
                      ) : topup.receipt_available && topup.lago_invoice_id ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() =>
                            void handleReceipt(topup.lago_invoice_id ?? "")
                          }
                        >
                          Download receipt
                        </Button>
                      ) : (
                        <span className="text-muted-foreground">—</span>
                      )}
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}

function WalletCard({
  wallet,
  loading,
  unavailable,
  error,
  onRetry,
  onProvision,
  provisioning,
  billingReady,
}: {
  readonly wallet: BillingWalletResponse | undefined;
  readonly loading: boolean;
  readonly unavailable: boolean;
  readonly error: unknown;
  readonly onRetry: () => void;
  readonly onProvision: () => void;
  readonly provisioning: boolean;
  readonly billingReady: boolean;
}) {
  if (loading) {
    return <Skeleton className="h-[278px] w-full" />;
  }

  if (!wallet && unavailable) {
    return (
      <Card>
        <CardHeader className="flex-row items-center justify-between space-y-0">
          <div>
            <CardTitle>Wallet</CardTitle>
            <p className="mt-1 text-[12px] text-muted-foreground">
              No wallet provisioned.
            </p>
          </div>
          <WalletCards className="h-4 w-4 text-text-tertiary" />
        </CardHeader>
        <CardContent>
          <Button
            variant="primary"
            disabled={!billingReady || provisioning}
            isLoading={provisioning}
            onClick={onProvision}
          >
            <ButtonIcon variant="primary">
              <Plus className="h-3 w-3" />
            </ButtonIcon>
            Provision Wallet
          </Button>
        </CardContent>
      </Card>
    );
  }

  if (!wallet && error) {
    return (
      <ErrorBanner
        message={errorMessage(error, "Failed to load billing wallet.")}
        onRetry={onRetry}
      />
    );
  }

  if (!wallet) {
    return null;
  }

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <div>
          <CardTitle>Wallet</CardTitle>
          <p className="mt-1 text-[12px] text-muted-foreground">
            Owner {wallet.owner_id}
          </p>
        </div>
        <Badge variant={wallet.suspended ? "destructive" : "success"}>
          {labelize(wallet.collection_state)}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-3 sm:grid-cols-3">
          <MetricBlock label="Available" value={formatCredits(wallet.available_credits)} />
          <MetricBlock label="Balance" value={formatCredits(wallet.balance_credits)} />
          <MetricBlock label="Reserved" value={formatCredits(wallet.reserved_credits)} />
        </div>
        <div className="grid gap-3 sm:grid-cols-3">
          <Detail label="Plan" value={labelize(wallet.plan_kind)} />
          <Detail label="Overdraft" value={formatCredits(wallet.overdraft_cap_credits)} />
          <Detail label="Synced" value={formatDateTime(wallet.balance_synced_at)} />
        </div>
      </CardContent>
    </Card>
  );
}

function UsageSummary({
  rows,
  totals,
  loading,
}: {
  readonly rows: readonly BillingUsageRow[];
  readonly totals:
    | {
        readonly quantity: number;
        readonly requests: number;
        readonly bytes: number;
        readonly events: number;
        readonly estimated_credits_micros?: number | null;
      }
    | undefined;
  readonly loading: boolean;
}) {
  const groupedRows = useMemo(() => rows, [rows]);

  if (loading) {
    return <Skeleton className="h-[320px] w-full" />;
  }

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <div>
          <CardTitle>Usage</CardTitle>
          <p className="mt-1 text-[12px] text-muted-foreground">
            Per-service quantity and estimated cost.
          </p>
        </div>
        <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
          <RefreshCw className="h-3.5 w-3.5" />
          {formatEstimatedCredits(totals?.estimated_credits_micros)}
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-3 sm:grid-cols-4">
          <MetricBlock label="Quantity" value={formatNumber(totals?.quantity ?? 0)} />
          <MetricBlock label="Requests" value={formatNumber(totals?.requests ?? 0)} />
          <MetricBlock label="Bytes" value={formatNumber(totals?.bytes ?? 0)} />
          <MetricBlock label="Events" value={formatNumber(totals?.events ?? 0)} />
        </div>
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Service</TableHead>
                <TableHead>Layer</TableHead>
                <TableHead>Metric</TableHead>
                <TableHead className="text-right">Quantity</TableHead>
                <TableHead className="text-right">Cost</TableHead>
                <TableHead>Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {groupedRows.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                    No usage in this period.
                  </TableCell>
                </TableRow>
              ) : (
                groupedRows.map((row, index) => (
                  <TableRow key={`${row.service_id ?? row.service_slug ?? "service"}-${row.layer}-${row.metric}-${index}`}>
                    <TableCell className="font-medium">
                      {row.service_slug ?? row.service_id ?? "Unknown"}
                    </TableCell>
                    <TableCell>{labelize(row.layer)}</TableCell>
                    <TableCell>{labelize(row.metric)}</TableCell>
                    <TableCell className="text-right">{formatNumber(row.quantity)}</TableCell>
                    <TableCell className="text-right">
                      {row.billable === false
                        ? "—"
                        : formatEstimatedCredits(row.estimated_credits_micros)}
                    </TableCell>
                    <TableCell>
                      {row.billable === false ? (
                        <Badge variant="secondary">Free</Badge>
                      ) : (
                        <Badge variant={row.lago_acked ? "success" : "secondary"}>
                          {row.lago_acked ? "Acked" : "Pending"}
                        </Badge>
                      )}
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}

function MetricBlock({ label, value }: { readonly label: string; readonly value: string }) {
  return (
    <div className="rounded-lg border border-border/70 bg-overlay px-3 py-3">
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className="mt-1 truncate text-[20px] font-semibold leading-tight">{value}</div>
    </div>
  );
}

function Detail({ label, value }: { readonly label: string; readonly value: string }) {
  return (
    <div>
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className="mt-1 text-[12px] text-foreground">{value}</div>
    </div>
  );
}

function periodLabel(period: BillingUsagePeriod): string {
  switch (period) {
    case "24h":
    case "1d":
      return "24 hours";
    case "7d":
      return "7 days";
    case "30d":
      return "30 days";
    case "90d":
      return "90 days";
    case "all":
      return "All time";
  }
}

function labelize(value: string): string {
  return value
    .split(/[_-]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function formatCredits(value: number): string {
  return `${formatNumber(value)} credits`;
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

function formatEstimatedCredits(value: number | null | undefined): string {
  if (value === null || value === undefined) {
    return "-";
  }
  return `${new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 6,
  }).format(value / 1_000_000)} credits`;
}

function formatDateTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "-";
  }
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

function isBillingNotConfigured(error: unknown): boolean {
  return error instanceof ApiError && error.errorCode === 11301;
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return fallback;
}
