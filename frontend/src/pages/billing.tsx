import { useState } from "react";
import type { ReactNode } from "react";
import {
  CircleDollarSign,
  Clock3,
  ReceiptText,
  RefreshCw,
  WalletCards,
} from "lucide-react";
import { useBillingUsage, useBillingWallet } from "@/hooks/use-billing";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type {
  BillingUsageResponse,
  BillingUsageRow,
  BillingWalletResponse,
} from "@/schemas/billing";
import { cn } from "@/lib/utils";

const PERIODS = [
  { value: "24h", label: "24h" },
  { value: "7d", label: "7d" },
  { value: "30d", label: "30d" },
  { value: "90d", label: "90d" },
  { value: "all", label: "All" },
] as const;

export function BillingPage() {
  const [period, setPeriod] = useState("30d");
  const wallet = useBillingWallet();
  const usage = useBillingUsage(period);

  const refresh = () => {
    void wallet.refetch();
    void usage.refetch();
  };

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Billing"
        description="Credits, invoices, and per-service billing usage."
        leading={
          <div className="flex h-10 w-10 items-center justify-center rounded-lg border border-white/[0.08] bg-white/[0.04]">
            <WalletCards className="h-4 w-4 text-nyx-secondary-400" />
          </div>
        }
        actions={
          <Button
            variant="outline"
            onClick={refresh}
            isLoading={wallet.isFetching || usage.isFetching}
          >
            <ButtonIcon>
              <RefreshCw className="h-3 w-3" />
            </ButtonIcon>
            Refresh
          </Button>
        }
      />

      {(wallet.error || usage.error) && (
        <ErrorBanner
          message={
            wallet.error instanceof Error
              ? wallet.error.message
              : usage.error instanceof Error
                ? usage.error.message
                : "Failed to load billing data."
          }
          onRetry={refresh}
        />
      )}

      <div className="grid gap-4 xl:grid-cols-[360px_minmax(0,1fr)]">
        <WalletPanel
          wallet={wallet.data}
          isLoading={wallet.isLoading}
        />
        <UsagePanel
          usage={usage.data}
          period={period}
          onPeriodChange={setPeriod}
          isLoading={usage.isLoading}
        />
      </div>
    </div>
  );
}

function WalletPanel({
  wallet,
  isLoading,
}: {
  readonly wallet: BillingWalletResponse | undefined;
  readonly isLoading: boolean;
}) {
  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between gap-3">
            <CardTitle>Credits</CardTitle>
            {wallet && (
              <Badge variant={wallet.wallet_configured ? "success" : "secondary"}>
                {wallet.status.replaceAll("_", " ")}
              </Badge>
            )}
          </div>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <div className="space-y-3">
              <Skeleton className="h-9 w-40" />
              <Skeleton className="h-4 w-full" />
              <Skeleton className="h-4 w-3/4" />
            </div>
          ) : (
            <div className="space-y-5">
              <div>
                <div className="text-[11px] font-medium uppercase text-text-tertiary">
                  Available
                </div>
                <div className="mt-1 text-[28px] font-semibold leading-tight">
                  {formatCredits(wallet?.available_credits)}
                </div>
              </div>

              <div className="grid gap-3 text-[12px]">
                <DetailRow
                  label="Balance"
                  value={formatCredits(wallet?.balance_credits)}
                />
                <DetailRow
                  label="Reserved"
                  value={formatCredits(wallet?.reserved_credits)}
                />
                <DetailRow
                  label="Pending Lago debits"
                  value={formatCredits(wallet?.pending_lago_debits)}
                />
                <DetailRow
                  label="Lago"
                  value={wallet?.lago_configured ? "configured" : "not configured"}
                />
                <DetailRow
                  label="Charging"
                  value={wallet?.charging_enabled ? "enabled" : "disabled"}
                />
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center gap-2">
            <ReceiptText className="h-4 w-4 text-text-tertiary" />
            <CardTitle>Invoices</CardTitle>
          </div>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <Skeleton className="h-16 w-full" />
          ) : wallet?.invoices.length ? (
            <div className="space-y-3">
              {wallet.invoices.map((invoice) => (
                <div
                  key={invoice.id}
                  className="rounded-lg border border-border px-3 py-2 text-[12px]"
                >
                  <div className="flex items-center justify-between gap-3">
                    <span className="truncate font-medium">{invoice.id}</span>
                    <Badge variant="secondary">{invoice.status}</Badge>
                  </div>
                  <div className="mt-2 text-muted-foreground">
                    {formatMicros(invoice.amount_credits_micros)}
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className="rounded-lg border border-border bg-white/[0.02] px-3 py-4 text-[12px] text-muted-foreground">
              No invoices are available.
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function UsagePanel({
  usage,
  period,
  onPeriodChange,
  isLoading,
}: {
  readonly usage: BillingUsageResponse | undefined;
  readonly period: string;
  readonly onPeriodChange: (period: string) => void;
  readonly isLoading: boolean;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="grid gap-3 md:grid-cols-3">
        <SummaryCard
          icon={<CircleDollarSign className="h-4 w-4" />}
          label="Estimated credits"
          value={formatMicros(usage?.totals.estimated_credits_micros)}
          loading={isLoading}
        />
        <SummaryCard
          icon={<Clock3 className="h-4 w-4" />}
          label="Events"
          value={formatNumber(usage?.totals.events)}
          loading={isLoading}
        />
        <SummaryCard
          icon={<ReceiptText className="h-4 w-4" />}
          label="Requests"
          value={formatNumber(usage?.totals.requests)}
          loading={isLoading}
        />
      </div>

      <Card>
        <CardHeader className="pb-3">
          <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
            <div>
              <CardTitle>Per-Service Cost</CardTitle>
              <div className="mt-1 text-[12px] text-muted-foreground">
                {usage?.billing.rates_are_approximate
                  ? "Rate-cache estimates, not invoices."
                  : "Provider-reported billing data."}
              </div>
            </div>
            <div className="inline-flex w-fit rounded-lg border border-border bg-white/[0.03] p-1">
              {PERIODS.map((item) => (
                <button
                  key={item.value}
                  type="button"
                  onClick={() => onPeriodChange(item.value)}
                  className={cn(
                    "h-7 min-w-10 rounded-md px-2.5 text-[12px] transition-colors",
                    period === item.value
                      ? "bg-white/[0.08] text-foreground"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {item.label}
                </button>
              ))}
            </div>
          </div>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-9 w-full" />
              <Skeleton className="h-9 w-full" />
              <Skeleton className="h-9 w-full" />
            </div>
          ) : usage?.rows.length ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Service</TableHead>
                  <TableHead>Layer</TableHead>
                  <TableHead>Metric</TableHead>
                  <TableHead className="text-right">Quantity</TableHead>
                  <TableHead className="text-right">Events</TableHead>
                  <TableHead>Lago</TableHead>
                  <TableHead className="text-right">Credits</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {usage.rows.map((row) => (
                  <UsageRow key={rowKey(row)} row={row} />
                ))}
              </TableBody>
            </Table>
          ) : (
            <div className="rounded-lg border border-border bg-white/[0.02] px-4 py-8 text-center text-[12px] text-muted-foreground">
              No billing usage for this period.
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function SummaryCard({
  icon,
  label,
  value,
  loading,
}: {
  readonly icon: ReactNode;
  readonly label: string;
  readonly value: string;
  readonly loading: boolean;
}) {
  return (
    <Card>
      <CardContent className="flex items-center gap-3 p-4">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-white/[0.08] bg-white/[0.04] text-nyx-secondary-400">
          {icon}
        </div>
        <div className="min-w-0">
          <div className="text-[11px] font-medium uppercase text-text-tertiary">
            {label}
          </div>
          {loading ? (
            <Skeleton className="mt-2 h-5 w-20" />
          ) : (
            <div className="mt-1 truncate text-[18px] font-semibold">{value}</div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function UsageRow({ row }: { readonly row: BillingUsageRow }) {
  return (
    <TableRow>
      <TableCell className="font-medium">
        {row.service_slug ?? row.service_id ?? "-"}
      </TableCell>
      <TableCell>{row.layer}</TableCell>
      <TableCell>{row.metric}</TableCell>
      <TableCell className="text-right">{formatNumber(row.quantity)}</TableCell>
      <TableCell className="text-right">{formatNumber(row.events)}</TableCell>
      <TableCell>
        <Badge variant={row.lago_acked ? "success" : "secondary"}>
          {row.lago_acked ? "acked" : "pending"}
        </Badge>
      </TableCell>
      <TableCell className="text-right">
        {formatMicros(row.estimated_credits_micros)}
      </TableCell>
    </TableRow>
  );
}

function DetailRow({
  label,
  value,
}: {
  readonly label: string;
  readonly value: string;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <span className="text-muted-foreground">{label}</span>
      <span className="truncate text-right font-medium">{value}</span>
    </div>
  );
}

function formatCredits(value: number | null | undefined): string {
  return value === null || value === undefined ? "-" : value.toLocaleString();
}

function formatNumber(value: number | null | undefined): string {
  return value === null || value === undefined ? "0" : value.toLocaleString();
}

function formatMicros(value: number | null | undefined): string {
  if (value === null || value === undefined) return "-";
  return (value / 1_000_000).toLocaleString(undefined, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 6,
  });
}

function rowKey(row: BillingUsageRow): string {
  return [
    row.service_slug ?? row.service_id ?? "unknown",
    row.layer,
    row.metric,
    row.lago_metric_code,
    row.lago_acked ? "acked" : "pending",
  ].join(":");
}
