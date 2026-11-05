import { BillingAllowanceBenefits } from "./billing-allowance-benefits";
import { useState } from "react";
import { useBillingBenefits } from "@/hooks/use-billing-benefits";
import { useBillingWallet } from "@/hooks/use-billing";
import { ApiError } from "@/lib/api-client";
import {
  exactCredits,
  formatCreditAmount,
  availableCreditTotal,
} from "@/lib/credits";
import { billingMetricLabel } from "@/lib/billing-units";
import {
  serviceName,
  timestamp,
  type BillingCatalog,
} from "@/lib/billing-display";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import type { CreditGrant } from "@/schemas/billing-credits";

const money = formatCreditAmount;
function grantState(g: CreditGrant) {
  if (g.status !== "active") return g.status;
  if (g.expires_at && Date.parse(g.expires_at) <= Date.now()) return "expired";
  if (g.revoke_requested_at) return "reclaim pending";
  if (g.activation_state === "pending_activation") return "pending activation";
  return "active";
}

export function AvailableBenefits({
  catalog = [],
  ownerId,
}: {
  catalog?: BillingCatalog;
  ownerId?: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const [page, setPage] = useState(1);
  const benefits = useBillingBenefits(page, ownerId);
  const wallet = useBillingWallet(ownerId);
  if (benefits.isLoading || wallet.isLoading)
    return (
      <Skeleton aria-label="Loading available credits" className="mt-6 h-36" />
    );
  const noWallet =
    wallet.data === null ||
    (wallet.error instanceof ApiError && wallet.error.errorCode === 11301);
  const denied = [benefits.error, wallet.error].some(
    (e) => e instanceof ApiError && e.status === 403,
  );
  const data = denied ? undefined : benefits.data;
  const walletAvailable = wallet.data
    ? exactCredits(
        wallet.data.available,
        wallet.data.available_credits,
        "whole",
      )
    : noWallet
      ? "0"
      : null;
  const suspended =
    wallet.data?.suspended || wallet.data?.collection_state === "suspended";
  const blocked = suspended || noWallet;
  const complete = Boolean(data && walletAvailable !== null);
  const stale = benefits.isError || (wallet.isError && !noWallet);
  const retry = () => {
    void benefits.refetch();
    void wallet.refetch();
  };
  return (
    <Card
      className="mt-6"
      aria-label="Current available benefits"
      aria-busy={benefits.isFetching || wallet.isFetching}
    >
      <CardHeader>
        {noWallet && (
          <p role="status" className="text-12 text-warning">
            Provision a wallet to use your grants and allowances.
          </p>
        )}
        {suspended && (
          <p role="alert" className="text-12 font-medium text-warning">
            Usage is suspended. Wallet credits, grants, and allowances cannot
            fund requests until billing is restored.
          </p>
        )}
        <CardTitle>Available credits</CardTitle>
        <p
          className="font-display text-28 font-medium tabular-nums"
          title={
            complete
              ? blocked
                ? "0"
                : availableCreditTotal(
                    walletAvailable!,
                    data!.summary.unreserved,
                  )
              : undefined
          }
        >
          {complete
            ? money(
                blocked
                  ? "0"
                  : availableCreditTotal(
                      walletAvailable!,
                      data!.summary.unreserved,
                    ),
              )
            : "Unavailable"}
        </p>
        <p className="text-12 text-muted-foreground">
          Wallet plus spendable grants. Grants apply to their eligible services;
          free allowances are additional usage in their own units. Current
          balances are independent of usage filters.
        </p>
        {complete && blocked && (
          <p className="text-12 text-muted-foreground">
            Balance before billing restrictions:{" "}
            {money(
              availableCreditTotal(walletAvailable!, data!.summary.unreserved),
            )}
            .
          </p>
        )}
        {stale && (
          <ErrorBanner
            message={
              complete
                ? "Showing previously loaded balances. Refresh failed."
                : "Could not load complete available balances."
            }
            onRetry={retry}
          />
        )}
        {!stale && (benefits.isFetching || wallet.isFetching) && (
          <p role="status" className="text-11 text-text-tertiary">
            Updating balances…
          </p>
        )}
      </CardHeader>
      {data && (
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-x-6 gap-y-2 text-12">
            <span>
              Wallet:{" "}
              <strong>
                {walletAvailable === null
                  ? "Unavailable"
                  : money(walletAvailable)}
              </strong>
            </span>
            <span>
              Grants unreserved:{" "}
              <strong>{money(data.summary.unreserved)}</strong>
            </span>
            <span>
              Free usage:{" "}
              <strong>
                {data.allowances.length}{" "}
                {data.allowances.length === 1 ? "allowance" : "allowances"}
              </strong>
            </span>
          </div>
          {data.allowances.length > 0 && (
            <ul
              aria-label="Available free usage"
              className="flex flex-wrap gap-x-5 gap-y-1 text-12 text-muted-foreground"
            >
              {data.allowances.slice(0, 3).map((a) => (
                <li key={a.allowance.id}>
                  {serviceName(catalog, a.allowance.service_slug)}:{" "}
                  {a.remaining_quantity.toLocaleString()}{" "}
                  {billingMetricLabel(a.allowance.metric)} left
                </li>
              ))}
              {data.allowances.length > 3 && (
                <li>+{data.allowances.length - 3} more in balance breakdown</li>
              )}
            </ul>
          )}
          <details
            open={expanded}
            onToggle={(e) => setExpanded(e.currentTarget.open)}
          >
            <summary className="w-fit cursor-pointer text-12 font-medium">
              Balance breakdown
            </summary>
            <dl className="my-3 grid gap-2 text-12 sm:grid-cols-2">
              {(
                [
                  ["Held grants", data.summary.held],
                  ["Pending activation", data.summary.pending_activation],
                  ["Pending reclaim", data.summary.pending_reclaim],
                  [
                    "Expired, awaiting cleanup",
                    data.summary.expired_awaiting_cleanup,
                  ],
                  [
                    "Available with service restrictions",
                    data.summary.restricted,
                  ],
                ] as const
              ).map(([label, value]) => (
                <div key={label}>
                  <dt className="text-muted-foreground">{label}</dt>
                  <dd className="tabular-nums">{money(value)}</dd>
                </div>
              ))}
            </dl>
            <h3 className="text-13 font-semibold">Individual grants</h3>
            <p className="my-1 text-11 text-text-tertiary">
              {data.total_grants.toLocaleString()}{" "}
              {data.total_grants === 1 ? "grant" : "grants"} · page {data.page}.
              The available total includes every eligible grant, across all
              pages.
            </p>
            <ul className="divide-y divide-border/50">
              {data.grants.map((g) => (
                <li
                  key={g.id}
                  className="min-w-0 space-y-1 break-words py-3 text-12"
                >
                  <div className="flex flex-wrap justify-between gap-2">
                    <strong className="min-w-0 break-all">
                      {g.reason || "Credit grant"}
                    </strong>
                    <span className="capitalize text-muted-foreground">
                      {grantState(g)}
                    </span>
                  </div>
                  <p className="tabular-nums">
                    Available:{" "}
                    {money(
                      grantState(g) === "active" ? (g.available ?? "0") : "0",
                    )}{" "}
                    · Held:{" "}
                    {money(
                      g.held ??
                        exactCredits(g.reserved, g.reserved_micros) ??
                        "0",
                    )}{" "}
                    · Remaining:{" "}
                    {money(
                      exactCredits(g.remaining, g.remaining_micros) ?? "0",
                    )}
                  </p>
                  <p className="text-11 text-muted-foreground">
                    Granted{" "}
                    {money(exactCredits(g.amount, g.amount_micros) ?? "0")} ·
                    Issued {timestamp(g.created_at)}
                  </p>
                  <p className="text-muted-foreground">
                    {g.scope.all_services
                      ? "All services"
                      : g.scope.service_slugs
                          .map((s) => serviceName(catalog, s))
                          .join(", ")}{" "}
                    ·{" "}
                    {g.expires_at
                      ? `Expires ${timestamp(g.expires_at)}`
                      : "No expiry"}
                  </p>
                  {g.service_limits?.map((r) => (
                    <p key={r.key} className="text-11 text-muted-foreground">
                      {r.label ||
                        r.service_slugs
                          .map((s) => serviceName(catalog, s))
                          .join(", ")}
                      : {money(r.limit)} limit · {money(r.consumed)} used ·{" "}
                      {money(r.held ?? r.reserved)} held
                    </p>
                  ))}
                </li>
              ))}
            </ul>
            {!data.grants.length && (
              <p className="py-3 text-12 text-muted-foreground">
                No grants on this page.
              </p>
            )}
            {data.total_grants > data.per_page && (
              <div className="my-3 flex gap-2">
                <Button
                  variant="outline"
                  disabled={page === 1 || benefits.isFetching}
                  onClick={() => setPage(page - 1)}
                >
                  Previous grants
                </Button>
                <Button
                  variant="outline"
                  disabled={
                    page * data.per_page >= data.total_grants ||
                    benefits.isFetching
                  }
                  onClick={() => setPage(page + 1)}
                >
                  Next grants
                </Button>
              </div>
            )}
            <h3 className="mt-4 text-13 font-semibold">Free allowances</h3>
            <BillingAllowanceBenefits
              allowances={data.allowances}
              catalog={catalog}
            />
            <p className="mt-3 text-11 text-text-tertiary">
              Benefits refreshed {timestamp(data.summary.generated_at)}.
              Allowances apply first, then eligible grants, then wallet credits.
            </p>
          </details>
        </CardContent>
      )}
    </Card>
  );
}
