import { Fragment, useMemo, useState } from "react";
import { toast } from "sonner";
import {
  ChevronDown,
  ChevronRight,
  CreditCard,
  ExternalLink,
  Info,
  Plus,
  WalletCards,
} from "lucide-react";
import { ApiError } from "@/lib/api-client";
import { openExternal } from "@/lib/navigation";
import { formatRelativeTime } from "@/lib/utils";
import {
  BILLING_USAGE_PERIODS,
  type BillingMetric,
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
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
const TOP_UP_PRESETS = [100, 500, 1_000, 5_000] as const;

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

      {/* items-start: expanding the wallet breakdown must not stretch Top Up. */}
      <div className="grid items-start gap-4 xl:grid-cols-[1.2fr_0.8fr]">
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
              <CardTitle>Add credits</CardTitle>
              <p className="mt-1 text-[12px] text-muted-foreground">
                1 credit = 1 USD. Credits never expire.
              </p>
            </div>
            <CreditCard className="h-4 w-4 text-text-tertiary" />
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <span className="text-[12px] font-medium">Amount</span>
              <div className="flex flex-wrap gap-2">
                {TOP_UP_PRESETS.map((preset) => (
                  <Button
                    key={preset}
                    variant={
                      topUpAmount === preset && topUpCredits !== ""
                        ? "secondary"
                        : "outline"
                    }
                    size="sm"
                    disabled={!billingReady || topUpBilling.isPending}
                    onClick={() => setTopUpCredits(String(preset))}
                  >
                    {formatNumber(preset)}
                  </Button>
                ))}
              </div>
            </div>

            <div className="space-y-2">
              <label className="text-[12px] font-medium" htmlFor="topup-credits">
                Or enter an amount
              </label>
              <div className="flex items-center gap-3">
                <Input
                  id="topup-credits"
                  type="number"
                  min={1}
                  max={10_000_000}
                  step={1}
                  value={topUpCredits}
                  onChange={(event) => setTopUpCredits(event.target.value)}
                  disabled={!billingReady || topUpBilling.isPending}
                  aria-describedby="topup-total"
                />
                <span
                  id="topup-total"
                  className="shrink-0 text-[13px] text-muted-foreground"
                >
                  {formatUsd(topUpAmount)}
                </span>
              </div>
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
              Continue to payment
            </Button>
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              {billingReady
                ? "We'll hand you to Stripe to pay, then bring you back here. Credits land once the payment clears."
                : "Payments aren't enabled on this deployment yet."}
            </p>
          </CardContent>
        </Card>
      </div>

      <UsageSummary
        rows={usageQuery.data?.rows ?? []}
        totals={usageQuery.data?.totals}
        loading={usageQuery.isLoading}
      />

      <TopUpHistory key={period} period={period} />
    </div>
  );
}

const TOPUP_STATUS_VARIANT = {
  paid: "success",
  pending: "secondary",
  expired: "warning",
  failed: "destructive",
  voided: "secondary",
} as const;

const TOPUPS_PER_PAGE = 10;

function TopUpHistory({ period }: { readonly period: BillingUsagePeriod }) {
  const [page, setPage] = useState(1);
  const historyQuery = useTopUpHistory(page, TOPUPS_PER_PAGE, period);
  const topups = historyQuery.data?.topups ?? [];
  const total = historyQuery.data?.total || topups.length;
  const pageCount = Math.max(1, Math.ceil(total / TOPUPS_PER_PAGE));

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
        {pageCount > 1 && (
          <div className="mt-3 flex items-center justify-between text-[12px] text-muted-foreground">
            <span>
              Page {page} of {pageCount}
            </span>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1 || historyQuery.isFetching}
                onClick={() => setPage((current) => Math.max(1, current - 1))}
              >
                Previous
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= pageCount || historyQuery.isFetching}
                onClick={() =>
                  setPage((current) => Math.min(pageCount, current + 1))
                }
              >
                Next
              </Button>
            </div>
          </div>
        )}
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
  const [showBreakdown, setShowBreakdown] = useState(false);

  if (loading) {
    return <Skeleton className="h-[170px] w-full" />;
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
        <CardTitle>Wallet</CardTitle>
        {wallet.suspended && <Badge variant="destructive">Suspended</Badge>}
      </CardHeader>
      <CardContent>
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <span className="text-[11px] text-muted-foreground">Balance</span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    aria-label="What this balance means"
                    className="text-text-tertiary transition-colors hover:text-foreground"
                  >
                    <Info className="h-3 w-3" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="top" className="max-w-[260px] leading-relaxed">
                  Credits held with our billing provider — 1 credit is 1 USD. This
                  is a synced figure, refreshed periodically rather than read live,
                  so a top-up or very recent usage can take a few minutes to show.
                </TooltipContent>
              </Tooltip>
            </div>
            <div className="mt-1 truncate text-[28px] font-semibold leading-tight">
              {formatCredits(wallet.balance_credits)}
            </div>
            <div className="mt-1 text-[11px] text-muted-foreground">
              Updated {formatRelativeTime(wallet.balance_synced_at)}
            </div>
          </div>
          <Button
            variant="outline"
            size="icon"
            aria-expanded={showBreakdown}
            aria-controls="wallet-breakdown"
            aria-label={
              showBreakdown ? "Hide balance breakdown" : "Show balance breakdown"
            }
            onClick={() => setShowBreakdown((current) => !current)}
          >
            <ChevronDown
              className={`h-3.5 w-3.5 transition-transform ${
                showBreakdown ? "rotate-180" : ""
              }`}
            />
          </Button>
        </div>

        {showBreakdown && (
          <div id="wallet-breakdown">
            <hr className="my-4 border-border" />
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              Not all of your balance is spendable at any moment — requests in
              flight hold credits until they finish.
            </p>
            <div className="mt-3 space-y-1.5">
              <BreakdownRow
                label="Available"
                hint="Spendable right now"
                value={formatCredits(wallet.available_credits)}
                emphasis
              />
              <BreakdownRow
                label="Reserved"
                hint="Held for requests in flight"
                value={formatCredits(wallet.reserved_credits)}
              />
              <BreakdownRow
                label="Pending"
                hint="Charged, awaiting provider sync"
                value={formatCredits(wallet.pending_lago_debits)}
              />
              {wallet.overdraft_cap_credits > 0 && (
                <BreakdownRow
                  label="Overdraft"
                  hint="Extra capacity beyond your balance"
                  value={formatCredits(wallet.overdraft_cap_credits)}
                />
              )}
            </div>
            <hr className="my-3 border-border" />
            <div className="flex items-center justify-between text-[11px] text-muted-foreground">
              <span>Plan</span>
              <span className="text-foreground">{labelize(wallet.plan_kind)}</span>
            </div>
            <p className="mt-2 text-[11px] text-muted-foreground">
              Available = Balance − Reserved − Pending.
            </p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function BreakdownRow({
  label,
  hint,
  value,
  emphasis = false,
}: {
  readonly label: string;
  readonly hint: string;
  readonly value: string;
  readonly emphasis?: boolean;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <div className="min-w-0">
        <div className={emphasis ? "text-[12px] font-medium" : "text-[12px]"}>
          {label}
        </div>
        <div className="text-[11px] text-muted-foreground">{hint}</div>
      </div>
      <div
        className={
          emphasis
            ? "shrink-0 text-[13px] font-semibold"
            : "shrink-0 text-[13px] text-muted-foreground"
        }
      >
        {value}
      </div>
    </div>
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
  const services = useMemo(() => groupByService(rows), [rows]);
  const metricTotals = useMemo(() => sumByMetric(rows), [rows]);
  const [expanded, setExpanded] = useState<readonly string[]>([]);

  function toggle(key: string) {
    setExpanded((current) =>
      current.includes(key)
        ? current.filter((entry) => entry !== key)
        : [...current, key],
    );
  }

  if (loading) {
    return <Skeleton className="h-[320px] w-full" />;
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Usage</CardTitle>
        <p className="mt-1 text-[12px] text-muted-foreground">
          Estimated cost per service. Expand a row for the model, agent, and
          layer behind it.
        </p>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-3 sm:grid-cols-4">
          <MetricBlock
            label="Est. cost"
            value={formatEstimatedCredits(totals?.estimated_credits_micros)}
          />
          <MetricBlock label="Tokens" value={formatNumber(metricTotals.tokens)} />
          <MetricBlock label="Requests" value={formatNumber(metricTotals.requests)} />
          <MetricBlock label="Bytes" value={formatNumber(metricTotals.bytes)} />
        </div>
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Service</TableHead>
                <TableHead>Usage</TableHead>
                <TableHead className="text-right">Est. cost</TableHead>
                <TableHead>Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {services.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={4} className="py-8 text-center text-muted-foreground">
                    No usage in this period.
                  </TableCell>
                </TableRow>
              ) : (
                services.map((service) => {
                  const isOpen = expanded.includes(service.key);
                  return (
                    <Fragment key={service.key}>
                      <TableRow
                        className="cursor-pointer"
                        onClick={() => toggle(service.key)}
                      >
                        <TableCell className="font-medium">
                          <button
                            type="button"
                            aria-expanded={isOpen}
                            aria-label={`${isOpen ? "Collapse" : "Expand"} ${service.label}`}
                            className="flex items-center gap-1.5 text-left"
                            onClick={(event) => {
                              event.stopPropagation();
                              toggle(service.key);
                            }}
                          >
                            <ChevronRight
                              className={`h-3.5 w-3.5 shrink-0 text-text-tertiary transition-transform ${
                                isOpen ? "rotate-90" : ""
                              }`}
                            />
                            <span className="truncate">{service.label}</span>
                          </button>
                        </TableCell>
                        <TableCell className="text-muted-foreground">
                          {describeUsage(service)}
                        </TableCell>
                        <TableCell className="text-right tabular-nums">
                          {formatEstimatedCredits(service.costMicros)}
                        </TableCell>
                        <TableCell>
                          <UsageStatusBadge
                            billable={service.billable}
                            acked={service.allAcked}
                          />
                        </TableCell>
                      </TableRow>
                      {isOpen &&
                        service.rows.map((row, index) => (
                          <TableRow
                            key={`${service.key}-detail-${row.layer}-${row.metric}-${row.model ?? ""}-${row.api_key_id ?? ""}-${index}`}
                            className="bg-overlay hover:bg-overlay"
                          >
                            <TableCell className="py-2 pl-9">
                              <div className="flex flex-wrap items-center gap-1.5 text-[12px]">
                                <Badge variant="secondary">
                                  {labelize(row.layer)}
                                </Badge>
                                {row.model && <span>{row.model}</span>}
                                <span className="text-muted-foreground">
                                  {describeAgent(row)}
                                </span>
                              </div>
                              <div className="mt-0.5 text-[11px] text-text-tertiary">
                                {formatNumber(row.events)}{" "}
                                {row.events === 1 ? "event" : "events"} ·{" "}
                                {row.lago_metric_code}
                              </div>
                            </TableCell>
                            <TableCell className="py-2 align-top text-[12px] text-muted-foreground">
                              {formatNumber(row.quantity)} {row.metric}
                            </TableCell>
                            <TableCell className="py-2 text-right align-top text-[12px] tabular-nums">
                              {row.billable === false
                                ? "—"
                                : formatEstimatedCredits(
                                    row.estimated_credits_micros,
                                  )}
                            </TableCell>
                            <TableCell className="py-2 align-top">
                              <UsageStatusBadge
                                billable={row.billable !== false}
                                acked={row.lago_acked}
                              />
                            </TableCell>
                          </TableRow>
                        ))}
                    </Fragment>
                  );
                })
              )}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}

function UsageStatusBadge({
  billable,
  acked,
}: {
  readonly billable: boolean;
  readonly acked: boolean;
}) {
  if (!billable) {
    return <Badge variant="secondary">Free</Badge>;
  }
  return (
    <Badge variant={acked ? "success" : "secondary"}>
      {acked ? "Acked" : "Pending"}
    </Badge>
  );
}

type ServiceGroup = {
  readonly key: string;
  readonly label: string;
  readonly rows: readonly BillingUsageRow[];
  readonly costMicros: number | null;
  readonly metrics: readonly BillingMetric[];
  readonly quantity: number;
  readonly allAcked: boolean;
  readonly billable: boolean;
};

/**
 * Collapse the flat ledger rows into one row per service.
 *
 * The API already splits a service across layer, metric, model, agent, and ack
 * state, so a single service routinely spans several rows. Costs are summed
 * (credits are a common unit); quantities are only summed when every row shares
 * one metric, because tokens, requests, and bytes cannot be meaningfully added.
 */
function groupByService(
  rows: readonly BillingUsageRow[],
): readonly ServiceGroup[] {
  const groups = new Map<string, BillingUsageRow[]>();
  for (const row of rows) {
    const key = row.service_slug ?? row.service_id ?? "unknown";
    const existing = groups.get(key);
    if (existing) {
      existing.push(row);
    } else {
      groups.set(key, [row]);
    }
  }

  return [...groups.entries()].map(([key, groupRows]) => {
    const costs = groupRows
      .map((row) => row.estimated_credits_micros)
      .filter((value): value is number => typeof value === "number");
    const metrics = [...new Set(groupRows.map((row) => row.metric))];
    return {
      key,
      label: groupRows[0]?.service_slug ?? groupRows[0]?.service_id ?? "Unknown",
      rows: groupRows,
      costMicros: costs.length > 0 ? costs.reduce((a, b) => a + b, 0) : null,
      metrics,
      quantity: groupRows.reduce((total, row) => total + row.quantity, 0),
      allAcked: groupRows.every((row) => row.lago_acked),
      billable: groupRows.some((row) => row.billable !== false),
    };
  });
}

/** Per-metric totals, so unlike units are never added into one number. */
function sumByMetric(rows: readonly BillingUsageRow[]): Record<BillingMetric, number> {
  const totals: Record<BillingMetric, number> = {
    tokens: 0,
    requests: 0,
    bytes: 0,
  };
  for (const row of rows) {
    totals[row.metric] += row.quantity;
  }
  return totals;
}

function describeUsage(service: ServiceGroup): string {
  const [metric] = service.metrics;
  if (service.metrics.length === 1 && metric) {
    return `${formatNumber(service.quantity)} ${metric}`;
  }
  return `${String(service.metrics.length)} metrics`;
}

/**
 * A null `api_key_id` only tells us the request was not attributed to an agent
 * key — it could be browser session, service-account, delegated, or relay auth,
 * and the ledger does not record which. Say that, rather than guessing
 * "Session".
 */
function describeAgent(row: BillingUsageRow): string {
  if (row.api_key_name) return row.api_key_name;
  if (row.api_key_id) return "Unnamed key";
  return "No agent key";
}

function MetricBlock({ label, value }: { readonly label: string; readonly value: string }) {
  return (
    <div className="rounded-lg border border-border/70 bg-overlay px-3 py-3">
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className="mt-1 truncate text-[20px] font-semibold leading-tight">{value}</div>
    </div>
  );
}

function periodLabel(period: BillingUsagePeriod): string {
  switch (period) {
    case "24h":
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

/** Credits are 1:1 with USD, so the charge is worth stating outright. */
function formatUsd(credits: number): string {
  if (!Number.isFinite(credits) || credits <= 0) {
    return "—";
  }
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: "USD",
  }).format(credits);
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

function isBillingNotConfigured(error: unknown): boolean {
  return error instanceof ApiError && error.errorCode === 11301;
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return fallback;
}
