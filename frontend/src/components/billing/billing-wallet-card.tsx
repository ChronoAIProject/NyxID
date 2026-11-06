import { formatRelativeTime } from "@/lib/utils";
import { ApiError } from "@/lib/api-client";
import { Label } from "@/components/ui/label";
import { formatExactCredits, legacyCredits, hasCredits } from "@/lib/credits";
import { useState } from "react";
import { ExternalLink, Plus } from "lucide-react";
import type { BillingWalletResponse } from "@/schemas/billing";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button, ButtonIcon } from "@/components/ui/button";
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

interface BillingWalletActionsProps {
  readonly wallet: BillingWalletResponse | undefined;
  readonly loading: boolean;
  readonly unavailable: boolean;
  readonly error: unknown;
  readonly onRetry: () => void;
  readonly onProvision: () => void;
  readonly provisioning: boolean;
  readonly billingReady: boolean;
  readonly needsSubscription?: boolean;
  readonly onTopUp: (amountCredits: number) => Promise<void>;
  readonly topUpPending: boolean;
  /** Controls the Add credits dialog when set (e.g. a top-up deep link). */
  readonly topUpOpen?: boolean;
  readonly onTopUpOpenChange?: (open: boolean) => void;
}

/** Existing checkout and provisioning controls, shared by the balance summary. */
export function BillingWalletActions({
  wallet,
  loading,
  unavailable,
  error,
  onRetry,
  onProvision,
  provisioning,
  billingReady,
  needsSubscription = false,
  onTopUp,
  topUpPending,
  topUpOpen,
  onTopUpOpenChange,
}: BillingWalletActionsProps) {
  if (loading)
    return (
      <Skeleton aria-label="Loading wallet actions" className="h-8 w-28" />
    );
  if (
    error &&
    (!wallet || (error instanceof ApiError && error.status === 403))
  ) {
    if (!unavailable)
      return (
        <ErrorBanner
          message={errorMessage(error, "Failed to load billing wallet.")}
          onRetry={onRetry}
        />
      );
  }
  if (!wallet && unavailable)
    return (
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
    );
  if (!wallet) return null;
  if (needsSubscription)
    return (
      <div className="max-w-sm space-y-2">
        <p role="status" className="type-secondary text-muted-foreground">
          Finish setting up the organization subscription before adding credits
          or enabling guests.
        </p>
        <Button
          variant="primary"
          onClick={onProvision}
          disabled={!billingReady || provisioning}
          isLoading={provisioning}
        >
          Finish wallet setup
        </Button>
      </div>
    );
  return (
    <AddCreditsDialog
      billingReady={billingReady}
      pending={topUpPending}
      onTopUp={onTopUp}
      open={topUpOpen}
      onOpenChange={onTopUpOpenChange}
    />
  );
}

export function BillingWalletBreakdown({
  wallet,
}: {
  wallet: BillingWalletResponse;
}) {
  return (
    <section aria-label="Wallet breakdown">
      <h3 className="type-secondary-strong">Wallet</h3>
      <p className="mt-1 type-secondary text-muted-foreground">
        Updated {formatRelativeTime(wallet.balance_synced_at)}
      </p>
      <div className="my-3">
        <BreakdownRow
          label="Available"
          hint="Wallet credits available now"
          value={formatCredits(wallet.available ?? wallet.available_credits)}
        />
      </div>
      <p className="type-secondary text-muted-foreground">
        Not all of your balance is spendable at any moment — requests in flight
        hold credits until they finish.
      </p>
      <div className="mt-3 space-y-1.5">
        <BreakdownRow
          label="Balance"
          hint="Last provider-synced balance"
          value={formatCredits(wallet.balance ?? wallet.balance_credits)}
        />
        <BreakdownRow
          label="Reserved"
          hint="Held for requests in flight"
          value={formatCredits(wallet.reserved ?? wallet.reserved_credits)}
        />
        <BreakdownRow
          label="Pending"
          hint="Charged, awaiting provider sync"
          value={formatCredits(
            wallet.pending_debits ?? wallet.pending_lago_debits,
          )}
        />
        {hasCredits(
          wallet.pending_expiry ?? wallet.pending_topup_expiry_credits,
        ) && (
          <BreakdownRow
            label="Expiring"
            hint="Held while expired purchases are removed"
            value={formatCredits(
              wallet.pending_expiry ?? wallet.pending_topup_expiry_credits,
            )}
          />
        )}
        {hasCredits(wallet.overdraft_cap ?? wallet.overdraft_cap_credits) && (
          <BreakdownRow
            label="Overdraft"
            hint="Extra capacity beyond your balance"
            value={formatCredits(
              wallet.overdraft_cap ?? wallet.overdraft_cap_credits,
            )}
          />
        )}
      </div>
      <hr className="my-3 border-border" />
      <div className="flex items-center justify-between type-secondary text-muted-foreground">
        <span>Plan</span>
        <span className="text-foreground">{labelize(wallet.plan_kind)}</span>
      </div>
      <p className="mt-2 type-secondary text-muted-foreground">
        Available = Balance - Reserved - Pending - Expiring.
      </p>
    </section>
  );
}

function AddCreditsDialog({
  billingReady,
  pending,
  onTopUp,
  open: controlledOpen,
  onOpenChange,
}: {
  readonly billingReady: boolean;
  readonly pending: boolean;
  readonly onTopUp: (amountCredits: number) => Promise<void>;
  readonly open?: boolean;
  readonly onOpenChange?: (open: boolean) => void;
}) {
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false);
  const open = controlledOpen ?? uncontrolledOpen;
  const setOpen = (next: boolean) => {
    setUncontrolledOpen(next);
    onOpenChange?.(next);
  };
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
            <span className="type-row-title">Amount</span>
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
            <Label htmlFor="topup-credits">Or enter an amount</Label>
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
                className="min-w-0 break-all type-body tabular-nums text-muted-foreground"
              >
                {formatUsd(amount)}
              </span>
            </div>
          </div>
          <p className="type-secondary text-muted-foreground">
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
    <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
      <div className="min-w-0">
        <div className="type-body">{label}</div>
        <div className="type-secondary text-muted-foreground">{hint}</div>
      </div>
      <div className="min-w-0 break-all type-body tabular-nums text-muted-foreground">
        {value}
      </div>
    </div>
  );
}

function formatCredits(value: string | number): string {
  return `${formatExactCredits(typeof value === "string" ? value : legacyCredits(value, "whole"))} credits`;
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
