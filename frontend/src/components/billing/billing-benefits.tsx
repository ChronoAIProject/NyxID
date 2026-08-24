import { Coins, Gauge } from "lucide-react";
import {
  useActiveCreditGrants,
  useCurrentAllowances,
} from "@/hooks/use-billing-credits";
import { ApiError } from "@/lib/api-client";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

export function BillingBenefits() {
  const grantsQuery = useActiveCreditGrants();
  const allowancesQuery = useCurrentAllowances();
  const grants = grantsQuery.data?.grants ?? [];
  const allowances = allowancesQuery.data?.allowances ?? [];
  const rolloutHidden = [grantsQuery.error, allowancesQuery.error].every(
    isForbidden,
  );

  if (rolloutHidden) return null;
  if (grantsQuery.isLoading || allowancesQuery.isLoading) {
    return <Skeleton className="h-40 w-full" />;
  }
  const visibleError = [grantsQuery.error, allowancesQuery.error].find(
    (error) => error && !isForbidden(error),
  );
  if (visibleError) {
    return (
      <ErrorBanner
        message={errorMessage(visibleError, "Failed to load billing credits")}
        onRetry={() => {
          void grantsQuery.refetch();
          void allowancesQuery.refetch();
        }}
      />
    );
  }
  if (grants.length === 0 && allowances.length === 0) return null;

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <CardTitle>Included credits</CardTitle>
        <Coins className="h-4 w-4 text-text-tertiary" />
      </CardHeader>
      <CardContent className="grid gap-6 md:grid-cols-2">
        <section className="min-w-0">
          <div className="mb-2 flex items-center gap-2 text-[12px] font-medium">
            <Coins className="h-3.5 w-3.5 text-success" /> Credit grants
          </div>
          {grants.length === 0 ? (
            <p className="text-[12px] text-muted-foreground">
              No active grants.
            </p>
          ) : (
            <div className="divide-y divide-border">
              {grants.map((grant) => (
                <div
                  key={grant.id}
                  className="flex items-start justify-between gap-3 py-2 first:pt-0"
                >
                  <div className="min-w-0">
                    <p className="truncate text-[12px] font-medium">
                      {grant.scope.all_services
                        ? "All services"
                        : grant.scope.service_slugs.join(", ")}
                    </p>
                    <p className="text-[11px] text-muted-foreground">
                      {grant.expires_at
                        ? `Expires ${new Date(grant.expires_at).toLocaleString()}`
                        : "No expiry"}
                    </p>
                  </div>
                  <span className="shrink-0 text-[12px] font-medium">
                    {formatEstimatedCredits(grant.remaining_micros)}
                  </span>
                </div>
              ))}
            </div>
          )}
        </section>
        <section className="min-w-0">
          <div className="mb-2 flex items-center gap-2 text-[12px] font-medium">
            <Gauge className="h-3.5 w-3.5 text-info" /> Free usage
          </div>
          {allowances.length === 0 ? (
            <p className="text-[12px] text-muted-foreground">
              No active allowances.
            </p>
          ) : (
            <div className="divide-y divide-border">
              {allowances.map(
                ({ allowance, remaining_quantity, period_end }) => (
                  <div
                    key={allowance.id}
                    className="flex items-start justify-between gap-3 py-2 first:pt-0"
                  >
                    <div className="min-w-0">
                      <p className="truncate text-[12px] font-medium">
                        {allowance.service_slug}
                      </p>
                      <p className="text-[11px] text-muted-foreground">
                        {labelize(allowance.metric)}
                        {period_end
                          ? ` / resets ${new Date(period_end).toLocaleString()}`
                          : " / one time"}
                      </p>
                    </div>
                    <span className="shrink-0 text-[12px] font-medium">
                      {formatNumber(remaining_quantity)} left
                    </span>
                  </div>
                ),
              )}
            </div>
          )}
        </section>
      </CardContent>
    </Card>
  );
}

function isForbidden(error: unknown): boolean {
  return error instanceof ApiError && error.status === 403;
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function labelize(value: string): string {
  return value
    .split(/[_-]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

function formatEstimatedCredits(value: number): string {
  return `${new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 6,
  }).format(value / 1_000_000)} credits`;
}
