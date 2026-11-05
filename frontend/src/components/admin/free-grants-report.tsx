import { useState } from "react";
import { ApiError } from "@/lib/api-client";
import {
  freeGrantParams,
  useFreeGrantsReport,
} from "@/hooks/use-free-grants-report";
import type { AnalyticsFilters } from "@/schemas/usage-analytics";
import { formatCreditAmount } from "@/lib/credits";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";

export function FreeGrantsReport({
  filters,
  refreshKey = 0,
}: {
  filters: AnalyticsFilters;
  refreshKey?: number;
}) {
  const params = freeGrantParams(filters);
  return <Report key={`${params.toString()}:${refreshKey}`} params={params} />;
}
function Report({ params }: { params: URLSearchParams }) {
  const [page, setPage] = useState(1);
  const [expanded, setExpanded] = useState(false);
  const [window, setWindow] = useState<{ from: string; to: string } | null>(
    null,
  );
  const effective = new URLSearchParams(params);
  if (window) {
    effective.delete("period");
    effective.set("from", window.from);
    effective.set("to", window.to);
  }
  const query = useFreeGrantsReport(effective, page);
  const denied =
    query.error instanceof ApiError && [403, 404].includes(query.error.status);
  const data = denied ? undefined : query.data;
  return (
    <Card
      aria-label="Free grants issued"
      className="my-5"
      aria-busy={query.isFetching}
    >
      <CardHeader>
        {query.isFetching && data && (
          <p role="status" className="type-secondary text-text-tertiary">
            Updating issued grants…
          </p>
        )}
        <CardTitle>Free grants issued</CardTitle>
        <p className="type-secondary text-muted-foreground">
          Credits actually issued to billing accounts in the selected UTC
          window. Uses the time and Users / organizations filters. Request-user
          and service filters do not apply. Issuance is separate from consumed
          or remaining credits.
        </p>
        {query.isError && (
          <ErrorBanner
            message={
              data
                ? "Showing previously loaded issuance totals. Refresh failed."
                : "Could not load free-grant issuance."
            }
            onRetry={() => void query.refetch()}
          />
        )}
        {query.isPending ? (
          <Skeleton className="h-12" />
        ) : (
          data && (
            <div>
              <p
                className="font-display type-metric tabular-nums"
                title={data.issued}
              >
                {formatCreditAmount(data.issued)}
              </p>
              <p className="mt-1 type-secondary text-text-tertiary">
                {data.grants.toLocaleString()}{" "}
                {data.grants === 1 ? "grant" : "grants"} across{" "}
                {data.total_accounts.toLocaleString()} billing{" "}
                {data.total_accounts === 1 ? "account" : "accounts"} · complete
                filtered population
              </p>
            </div>
          )
        )}
      </CardHeader>
      {data && (
        <CardContent>
          <p className="mb-3 type-secondary text-text-tertiary">
            Issued from {utc(data.window.from)} to {utc(data.window.to)} (end
            exclusive).
            {window
              ? " Window held fixed while paging; Refresh starts a new report."
              : ""}{" "}
            Updated {utc(data.generated_at)}.
          </p>
          <details
            open={expanded}
            onToggle={(e) => setExpanded(e.currentTarget.open)}
          >
            <summary className="w-fit cursor-pointer type-button">
              Issued by account
            </summary>
            <ul className="mt-3 divide-y divide-border/50">
              {data.accounts.map((a) => (
                <li
                  key={a.owner_id}
                  className="flex flex-wrap items-start justify-between gap-3 py-3 type-secondary"
                >
                  <div className="min-w-0 flex-1">
                    <p className="break-all type-row-title">{a.label}</p>
                    <p className="type-secondary text-text-tertiary">
                      {a.account_kind === "org"
                        ? "Organization"
                        : a.account_kind === "person"
                          ? "Personal account"
                          : "Historical account"}{" "}
                      · {a.grants.toLocaleString()}{" "}
                      {a.grants === 1 ? "grant" : "grants"}
                    </p>
                    <p className="break-all font-mono type-secondary text-text-tertiary">
                      {a.owner_id}
                    </p>
                  </div>
                  <strong className="tabular-nums" title={a.issued}>
                    {formatCreditAmount(a.issued)} issued
                  </strong>
                </li>
              ))}
            </ul>
            {!data.accounts.length && (
              <p className="py-3 type-secondary text-muted-foreground">
                No grants issued in this window.
              </p>
            )}
            {data.total_accounts > data.per_page && (
              <div className="mt-3 flex flex-wrap items-center gap-3">
                <Button
                  variant="outline"
                  disabled={page === 1 || query.isFetching}
                  onClick={() => setPage(page - 1)}
                >
                  Previous accounts
                </Button>
                <span className="type-secondary text-text-tertiary">
                  Page {data.page} of{" "}
                  {Math.ceil(data.total_accounts / data.per_page)}
                </span>
                <Button
                  variant="outline"
                  disabled={
                    page * data.per_page >= data.total_accounts ||
                    query.isFetching
                  }
                  onClick={() => {
                    setWindow(data.window);
                    setPage(page + 1);
                  }}
                >
                  Next accounts
                </Button>
              </div>
            )}
          </details>
        </CardContent>
      )}
    </Card>
  );
}

function utc(value: string) {
  return new Date(value).toLocaleString(undefined, {
    timeZone: "UTC",
    timeZoneName: "short",
  });
}
