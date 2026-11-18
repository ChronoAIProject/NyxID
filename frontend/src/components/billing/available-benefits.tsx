import { useCappedItems } from "@/hooks/use-capped-items";
import { BILLING_FIELD_LABELS } from "@/lib/billing-sections";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { BillingAllowanceBenefits } from "./billing-allowance-benefits";
import { createContext, useContext, useState, type ReactNode } from "react";
import { BillingWalletBreakdown } from "./billing-wallet-card";
import { useBillingBenefits } from "@/hooks/use-billing-benefits";
import { useBillingWallet } from "@/hooks/use-billing";
import { ApiError } from "@/lib/api-client";
import {
  exactCredits,
  formatExactCredits,
  formatCreditAmount,
  availableCreditTotal,
} from "@/lib/credits";
import {
  serviceName,
  timestamp,
  type BillingCatalog,
} from "@/lib/billing-display";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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

const BenefitsContext = createContext<{
  benefits: ReturnType<typeof useBillingBenefits>;
  wallet: ReturnType<typeof useBillingWallet>;
  page: number;
  setPage: (page: number) => void;
} | null>(null);

/** A single query owner for all retained benefit pages, including pagination and polling. */
export function AvailableBenefitsProvider({ ownerId, children }: { ownerId?: string; children: ReactNode }) {
  const [page, setPage] = useState(1);
  const benefits = useBillingBenefits(page, ownerId);
  const wallet = useBillingWallet(ownerId);
  return <BenefitsContext value={{ benefits, wallet, page, setPage }}>{children}</BenefitsContext>;
}

/** Root disclosure summary from the existing query owner; never starts a read. */
export function AvailableBenefitsCount() {
  useUiTranslation();
  const { benefits } = useContext(BenefitsContext)!;
  return benefits.data && !benefits.error
    ? <span className="text-13 text-text-tertiary">{translateUi("{{count}} grants", { count: benefits.data.total_grants })}</span>
    : null;
}

export function AvailableBenefits({
  catalog = [],
  actions,
  view,
}: {
  catalog?: BillingCatalog;
  actions?: ReactNode;
  view: "summary" | "details";
}) {
  useUiTranslation();
  const { benefits, wallet, page, setPage } = useContext(BenefitsContext)!;
  const visibleGrants = useCappedItems(benefits.data?.grants ?? [], 3);
  if (benefits.isLoading || wallet.isLoading)
    return (
      <Card
        className="mt-6"
        aria-label={translateUi("Current available benefits")}
        aria-busy="true"
      >
        <CardContent className="flex flex-wrap items-start justify-between gap-3 p-4">
          <Skeleton
            aria-label={translateUi("Loading available credits")}
            className="h-20 min-w-32 flex-1"
          />
          {actions}
        </CardContent>
      </Card>
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
  const total = complete
    ? blocked
      ? "0"
      : availableCreditTotal(walletAvailable!, data!.summary.unreserved)
    : null;
  return (
    <Card
      className="benefits-container mt-6"
      aria-label={translateUi("Current available benefits")}
      aria-busy={benefits.isFetching || wallet.isFetching}
    >
      <CardContent className="p-4">
        {view === "summary" && <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="type-secondary text-muted-foreground">{translateUi(BILLING_FIELD_LABELS.availableCredits)}</h2>
            <p
              className="mt-1 break-words font-display type-metric tabular-nums"
              title={total ?? undefined}
            >
              {total === null ? (
                translateUi("Unavailable")
              ) : (
                <>
                  {formatExactCredits(total)}{" "}
                  <span className="type-secondary text-muted-foreground">
                    {money(total).endsWith(" credits") ? translateUi("credits") : translateUi("credit")}
                  </span>
                </>
              )}
            </p>

          </div>
          {actions}
        </div>}
        {noWallet && (
          <p role="status" className="mt-2 type-secondary text-warning">{translateUi("No wallet provisioned. Provision a wallet to use your credits and free usage.")}</p>
        )}
        {suspended && (
          <p role="alert" className="mt-2 type-secondary-emphasis text-warning">{translateUi("Usage is suspended until billing is restored.")}</p>
        )}
        {stale && (
          <ErrorBanner
            message={
              complete
                ? translateUi("Showing previously loaded balances. Refresh failed.")
                : translateUi("Could not load complete available balances.")
            }
            onRetry={retry}
          />
        )}
        {!stale && (benefits.isFetching || wallet.isFetching) && (
          <p role="status" className="type-secondary text-text-tertiary">{translateUi("Updating balances…")}</p>
        )}

        {data && view !== "summary" && (
          <div
            className="space-y-4"
          >
            <p className="type-secondary text-muted-foreground">{translateUi("Wallet and eligible grants contribute to available credits. Free allowances are additional usage in their own units. Current balances are independent of usage filters.")}</p>
            {complete && blocked && (
              <p className="type-secondary text-muted-foreground">{translateUi("Balance before billing restrictions:")}{" "}
                {money(
                  availableCreditTotal(
                    walletAvailable!,
                    data.summary.unreserved,
                  ),
                )}
                .
              </p>
            )}
            {wallet.data && <BillingWalletBreakdown wallet={wallet.data} />}
            {<>
            <section aria-label={translateUi("Grant balances")}>
              <h3 className="type-secondary-strong">{translateUi("Billing grants")}</h3>
              <p className="mt-1 type-secondary">{translateUi("Available: ")}{money(data.summary.unreserved)}
              </p>
              <dl className="my-3 grid gap-2 type-secondary sm:grid-cols-2">
                {(
                  [
                    [BILLING_FIELD_LABELS.heldGrants, data.summary.held],
                    [BILLING_FIELD_LABELS.pendingActivation, data.summary.pending_activation],
                    [BILLING_FIELD_LABELS.pendingReclaim, data.summary.pending_reclaim],
                    [
                      BILLING_FIELD_LABELS.expiredAwaitingCleanup,
                      data.summary.expired_awaiting_cleanup,
                    ],
                    [
                      BILLING_FIELD_LABELS.availableWithServiceRestrictions,
                      data.summary.restricted,
                    ],
                  ] as const
                ).map(([label, value]) => (
                  <div key={label}>
                    <dt className="text-muted-foreground">{translateUi(label)}</dt>
                    <dd className="tabular-nums">{money(value)}</dd>
                  </div>
                ))}
              </dl>
            </section>
            <h3 className="type-secondary-strong">{translateUi(BILLING_FIELD_LABELS.individualGrants)}</h3>
            <p className="my-1 type-secondary text-text-tertiary">
              {data.total_grants.toLocaleString(getFormattingLocale())}{" "}
              {data.total_grants === 1 ? translateUi("credit grant") : translateUi("credit grants")}{translateUi(" · page ")}{data.page}{translateUi(". The available total includes every eligible grant, across all pages. ")}</p>
            <ul className="divide-y divide-border/50">
              {visibleGrants.items.map((g) => (
                <li
                  key={g.id}
                  className="min-w-0 space-y-1 break-words py-3 type-secondary"
                >
                  <div className="flex flex-wrap justify-between gap-2">
                    <strong className="min-w-0 break-all">
                      {g.reason || translateUi("Credit grant")}
                    </strong>
                    <span className="capitalize text-muted-foreground">
                      {grantState(g)}
                    </span>
                  </div>
                  <p className="tabular-nums">{translateUi("Available:")}{" "}
                    {money(
                      grantState(g) === "active" ? (g.available ?? "0") : "0",
                    )}{" "}{translateUi("· Held:")}{" "}
                    {money(
                      g.held ??
                        exactCredits(g.reserved, g.reserved_micros) ??
                        "0",
                    )}{" "}{translateUi("· Remaining:")}{" "}
                    {money(
                      exactCredits(g.remaining, g.remaining_micros) ?? "0",
                    )}
                  </p>
                  <p className="type-secondary text-muted-foreground">{translateUi("Granted")}{" "}
                    {money(exactCredits(g.amount, g.amount_micros) ?? "0")}{translateUi(" · Issued ")}{timestamp(g.created_at)}
                  </p>
                  <p className="text-muted-foreground">
                    {g.scope.all_services
                      ? translateUi("All services")
                      : g.scope.service_slugs
                          .map((s) => serviceName(catalog, s))
                          .join(", ")}{" "}
                    ·{" "}
                    {g.expires_at
                      ? translateUi("Expires {{value1}}", { value1: timestamp(g.expires_at) })
                      : translateUi("No expiry")}
                  </p>
                  {g.service_limits?.map((r) => (
                    <p
                      key={r.key}
                      className="type-secondary text-muted-foreground"
                    >
                      {r.label ||
                        r.service_slugs
                          .map((s) => serviceName(catalog, s))
                          .join(", ")}
                      : {money(r.limit)}{translateUi(" limit · ")}{money(r.consumed)}{translateUi(" used ·")}{" "}
                      {money(r.held ?? r.reserved)}{translateUi(" held")}</p>
                  ))}
                </li>
              ))}
            </ul>
            {visibleGrants.remaining > 0 && <Button variant="ghost" onClick={visibleGrants.showAll}>{translateUi("Show all")}</Button>}
            {!data.grants.length && (
              <p className="py-3 type-secondary text-muted-foreground">{translateUi("No grants on this page.")}</p>
            )}
            {data.total_grants > data.per_page && (
              <div className="my-3 flex gap-2">
                <Button
                  variant="outline"
                  disabled={page === 1 || benefits.isFetching}
                  onClick={() => setPage(page - 1)}
                >{translateUi("Previous grants")}</Button>
                <Button
                  variant="outline"
                  disabled={
                    page * data.per_page >= data.total_grants ||
                    benefits.isFetching
                  }
                  onClick={() => setPage(page + 1)}
                >{translateUi("Next grants")}</Button>
              </div>
            )}
            </>}
            {<>
            <h3 className="mt-4 type-secondary-strong">{translateUi(BILLING_FIELD_LABELS.freeAllowances)}</h3>
            <BillingAllowanceBenefits
              allowances={data.allowances}
              catalog={catalog}
            /></>}

          </div>
        )}
        {view === "details" && data && <p className="mt-3 type-secondary text-text-tertiary">{translateUi("Benefits refreshed ")}{timestamp(data.summary.generated_at)}{translateUi(". Allowances apply first, then eligible grants, then wallet credits. ")}</p>}
      </CardContent>
    </Card>
  );
}
