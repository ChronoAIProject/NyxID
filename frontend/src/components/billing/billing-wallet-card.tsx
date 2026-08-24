import { useState } from "react";
import {
  ChevronDown,
  ExternalLink,
  Info,
  Plus,
  WalletCards,
} from "lucide-react";
import type { BillingWalletResponse } from "@/schemas/billing";
import { formatRelativeTime } from "@/lib/utils";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

const DEFAULT_TOP_UP_CREDITS = 100;
const TOP_UP_PRESETS = [100, 500, 1_000, 5_000] as const;

interface BillingWalletCardProps {
  readonly wallet: BillingWalletResponse | undefined;
  readonly loading: boolean;
  readonly unavailable: boolean;
  readonly error: unknown;
  readonly onRetry: () => void;
  readonly onProvision: () => void;
  readonly provisioning: boolean;
  readonly billingReady: boolean;
  readonly onTopUp: (amountCredits: number) => Promise<void>;
  readonly topUpPending: boolean;
}

export function BillingWalletCard({
  wallet,
  loading,
  unavailable,
  error,
  onRetry,
  onProvision,
  provisioning,
  billingReady,
  onTopUp,
  topUpPending,
}: BillingWalletCardProps) {
  const [showBreakdown, setShowBreakdown] = useState(false);

  if (loading) return <Skeleton className="h-[170px] w-full" />;
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
  if (!wallet) return null;

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
              <span className="text-[11px] text-muted-foreground">
                Available
              </span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    aria-label="What available credits means"
                    className="text-text-tertiary transition-colors hover:text-foreground"
                  >
                    <Info className="h-3 w-3" />
                  </button>
                </TooltipTrigger>
                <TooltipContent
                  side="top"
                  className="max-w-[260px] leading-relaxed"
                >
                  Credits spendable now after in-flight requests, unsettled
                  usage, and purchased-credit expiry holds. 1 credit is 1 USD.
                </TooltipContent>
              </Tooltip>
            </div>
            <div className="mt-1 truncate text-[28px] font-semibold leading-tight">
              {formatCredits(wallet.available_credits)}
            </div>
            <div className="mt-1 text-[11px] text-muted-foreground">
              Updated {formatRelativeTime(wallet.balance_synced_at)}
            </div>
            <button
              type="button"
              aria-expanded={showBreakdown}
              aria-controls="wallet-breakdown"
              className="mt-2 flex items-center gap-1 text-[11px] text-muted-foreground transition-colors hover:text-foreground"
              onClick={() => setShowBreakdown((current) => !current)}
            >
              {showBreakdown ? "Hide breakdown" : "View breakdown"}
              <ChevronDown
                className={`h-3 w-3 transition-transform ${showBreakdown ? "rotate-180" : ""}`}
              />
            </button>
          </div>
          <AddCreditsDialog
            billingReady={billingReady}
            pending={topUpPending}
            onTopUp={onTopUp}
          />
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
                label="Balance"
                hint="Last provider-synced balance"
                value={formatCredits(wallet.balance_credits)}
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
              {wallet.pending_topup_expiry_credits > 0 && (
                <BreakdownRow
                  label="Expiring"
                  hint="Held while expired purchases are removed"
                  value={formatCredits(wallet.pending_topup_expiry_credits)}
                />
              )}
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
              <span className="text-foreground">
                {labelize(wallet.plan_kind)}
              </span>
            </div>
            <p className="mt-2 text-[11px] text-muted-foreground">
              Available = Balance - Reserved - Pending - Expiring.
            </p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function AddCreditsDialog({
  billingReady,
  pending,
  onTopUp,
}: {
  readonly billingReady: boolean;
  readonly pending: boolean;
  readonly onTopUp: (amountCredits: number) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [credits, setCredits] = useState(String(DEFAULT_TOP_UP_CREDITS));
  const amount = Number(credits);
  const amountValid =
    Number.isInteger(amount) && amount > 0 && amount <= 10_000_000;
  const trigger = (
    <Button variant="primary" disabled={!billingReady || pending}>
      <ButtonIcon variant="primary">
        <Plus className="h-3 w-3" />
      </ButtonIcon>
      Add credits
    </Button>
  );

  if (!billingReady) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span tabIndex={0}>{trigger}</span>
        </TooltipTrigger>
        <TooltipContent side="left">
          Payments aren&apos;t enabled on this deployment yet.
        </TooltipContent>
      </Tooltip>
    );
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>Add credits</DialogTitle>
          <DialogDescription>
            1 credit = 1 USD. Purchased credits expire one year after payment.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-2">
            <span className="text-[12px] font-medium">Amount</span>
            <div className="flex flex-wrap gap-2">
              {TOP_UP_PRESETS.map((preset) => (
                <Button
                  key={preset}
                  variant={amount === preset ? "secondary" : "outline"}
                  size="sm"
                  disabled={pending}
                  onClick={() => setCredits(String(preset))}
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
                value={credits}
                onChange={(event) => setCredits(event.target.value)}
                disabled={pending}
                aria-describedby="topup-total"
              />
              <span
                id="topup-total"
                className="shrink-0 text-[13px] text-muted-foreground"
              >
                {formatUsd(amount)}
              </span>
            </div>
          </div>
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            We&apos;ll hand you to Stripe to pay, then bring you back here.
            Credits land once the payment clears.
          </p>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={!billingReady || !amountValid || pending}
            isLoading={pending}
            onClick={() => void onTopUp(amount)}
          >
            <ButtonIcon variant="primary">
              <ExternalLink className="h-3 w-3" />
            </ButtonIcon>
            Continue to payment
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function BreakdownRow({
  label,
  hint,
  value,
}: {
  readonly label: string;
  readonly hint: string;
  readonly value: string;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <div className="min-w-0">
        <div className="text-[12px]">{label}</div>
        <div className="text-[11px] text-muted-foreground">{hint}</div>
      </div>
      <div className="shrink-0 text-[13px] text-muted-foreground">{value}</div>
    </div>
  );
}

function formatCredits(value: number): string {
  return `${new Intl.NumberFormat().format(value)} credits`;
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

function formatUsd(credits: number): string {
  if (!Number.isFinite(credits) || credits <= 0) return "—";
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: "USD",
  }).format(credits);
}

function labelize(value: string): string {
  return value
    .split(/[_-]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}
