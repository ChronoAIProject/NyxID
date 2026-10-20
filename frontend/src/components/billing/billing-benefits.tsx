import {
  useActiveCreditGrants,
  useCurrentAllowances,
} from "@/hooks/use-billing-credits";
import { ApiError } from "@/lib/api-client";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Skeleton } from "@/components/ui/skeleton";

import { BillingBenefitsCard } from "./billing-benefits-card";
import type { BillingCatalog } from "@/lib/billing-display";

export function BillingBenefits({
  catalog = [],
}: {
  catalog?: BillingCatalog;
}) {
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
    <BillingBenefitsCard
      grants={grants}
      allowances={allowances}
      catalog={catalog}
    />
  );
}

function isForbidden(error: unknown): boolean {
  return error instanceof ApiError && error.status === 403;
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}
