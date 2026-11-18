import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useId, useState } from "react";
import { ChevronDown } from "lucide-react";
import { ApiError } from "@/lib/api-client";
import {
  freeGrantParams,
  useFreeGrantsReport,
} from "@/hooks/use-free-grants-report";
import type { AnalyticsFilters } from "@/schemas/usage-analytics";
import { formatCreditAmount, formatExactCredits } from "@/lib/credits";
import { GrantCreditsTrend } from "./grant-credits-trend";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";

export function FreeGrantsReport({
  filters,
  refreshKey = 0,
}: {
  filters: AnalyticsFilters;
  refreshKey?: number;
}) {
  useUiTranslation();
  const params = freeGrantParams(filters);
  return <Report key={`${params.toString()}:${refreshKey}`} params={params} />;
}
function Report({ params }: { params: URLSearchParams }) {
  useUiTranslation();
  const id = useId();
  const [page, setPage] = useState(1);
  const [servicePage, setServicePage] = useState(1);
  const [expanded, setExpanded] = useState(false);
  const [mode, setMode] = useState("account");
  const [window, setWindow] = useState<{ from: string; to: string } | null>(
    null,
  );
  const effective = new URLSearchParams(params);
  if (window) {
    effective.delete("period");
    effective.set("from", window.from);
    effective.set("to", window.to);
  }
  const query = useFreeGrantsReport(effective, page, servicePage);
  const denied =
    query.error instanceof ApiError && [403, 404].includes(query.error.status);
  const data = denied ? undefined : query.data;
  const accountMode = mode === "account";
  const total = data
    ? accountMode
      ? data.total_accounts
      : data.total_services
    : 0;
  const currentPage = data ? (accountMode ? data.page : data.service_page) : 1;
  function navigate(next: number) {
    if (!data) return;
    setWindow(data.window);
    if (accountMode) setPage(next);
    else setServicePage(next);
  }
  return (
    <Card
      aria-label={translateUi("Grant credits")}
      className="my-5"
      aria-busy={query.isFetching}
    >
      <CardHeader>
        <CardTitle>{translateUi("Grant credits")}</CardTitle>
        <p className="type-secondary text-muted-foreground">{translateUi("Time and billing account filters · All services")}</p>
        {query.isError && (
          <ErrorBanner
            message={
              data
                ? translateUi("Showing previously loaded grant totals. Refresh failed.")
                : translateUi("Could not load grant credits.")
            }
            onRetry={() => void query.refetch()}
          />
        )}
        {query.isFetching && data && (
          <p role="status" className="type-secondary text-text-tertiary">{translateUi("Updating grant credits…")}</p>
        )}
        {query.isPending ? (
          <Skeleton className="h-16" />
        ) : (
          data && (
            <dl
              aria-label={translateUi("Complete grant credit totals")}
              className="flex flex-wrap gap-x-10 gap-y-3 pt-2"
            >
              {(
                [
                  ["Granted", data.issued],
                  ["Used", data.used],
                ] as const
              ).map(([label, value]) => (
                <div key={label} className="min-w-0 flex-1 basis-48">
                  <dt className="type-secondary text-muted-foreground">
                    {label}
                  </dt>
                  <dd
                    className="mt-1 break-words font-display type-metric tabular-nums"
                    title={value}
                  >
                    {formatExactCredits(value)}{" "}
                    <span className="type-secondary text-muted-foreground">
                      {formatCreditAmount(value).endsWith(" credits")
                        ? translateUi("credits")
                        : translateUi("credit")}
                    </span>
                  </dd>
                </div>
              ))}
            </dl>
          )
        )}
      </CardHeader>
      {data && (
        <CardContent className="space-y-3">
          <GrantCreditsTrend data={data} />
          <p className="type-secondary text-muted-foreground">{translateUi("Usage can include older grants and exceed credits granted in this window.")}</p>
          <p className="type-secondary text-text-tertiary">
            {utc(data.window.from)} – {utc(data.window.to)}{translateUi(" (end exclusive). Updated ")}{utc(data.generated_at)}.
            {window
              ? translateUi(" Window held fixed while paging; Refresh starts a new report.")
              : ""}
          </p>
          <button
            type="button"
            aria-expanded={expanded}
            aria-controls={id}
            onClick={() => setExpanded(!expanded)}
            className="flex min-h-6 items-center gap-1 type-row-title text-muted-foreground transition-colors hover:text-foreground"
          >
            {expanded ? translateUi("Hide breakdown") : translateUi("View breakdown")}
            <ChevronDown
              aria-hidden="true"
              className={`h-3 w-3 transition-transform ${expanded ? "rotate-180" : ""}`}
            />
          </button>
          <div
            id={id}
            hidden={!expanded}
            className="space-y-3 border-t border-border pt-3"
          >
            <p className="type-secondary text-muted-foreground">{translateUi("Uses the time and Users / organizations filters. Request-user and service filters do not apply.")}</p>
            <p className="type-secondary text-text-tertiary">
              {data.grants.toLocaleString(getFormattingLocale())}{" "}
              {data.grants === 1 ? translateUi("grant") : translateUi("grants")} ·{" "}
              {data.total_accounts.toLocaleString(getFormattingLocale())}{translateUi(" billing")}{" "}
              {data.total_accounts === 1 ? translateUi("account") : translateUi("accounts")}{translateUi(" with issuance or usage · complete filtered population")}</p>
            <Tabs value={mode} onValueChange={setMode}>
              <TabsList aria-label={translateUi("Grant credits breakdown")}>
                <TabsTrigger value="account">{translateUi("Account")}</TabsTrigger>
                <TabsTrigger value="service">{translateUi("Service")}</TabsTrigger>
              </TabsList>
              <TabsContent value="account">
                <ul
                  className="divide-y divide-border/50"
                  aria-label={translateUi("Grant credits by billing account")}
                >
                  {data.accounts.map((a) => (
                    <li
                      key={a.owner_id}
                      className="flex flex-wrap items-start justify-between gap-3 py-3 type-secondary"
                    >
                      <div className="min-w-0 flex-1 basis-40">
                        <p className="break-words type-row-title">{a.label}</p>
                        <p className="text-text-tertiary">
                          {a.account_kind === "org"
                            ? translateUi("Organization")
                            : a.account_kind === "person"
                              ? translateUi("Personal account")
                              : translateUi("Historical account")}{" "}
                          · {a.grants.toLocaleString(getFormattingLocale())}{" "}
                          {a.grants === 1 ? translateUi("grant") : translateUi("grants")}
                        </p>
                        <p className="break-all font-mono type-secondary text-text-tertiary">
                          {a.owner_id}
                        </p>
                      </div>
                      <dl className="flex max-w-full flex-wrap gap-x-5 gap-y-2">
                        <div className="min-w-0">
                          <dt className="text-muted-foreground">{translateUi("Granted")}</dt>
                          <dd
                            className="break-words tabular-nums"
                            title={a.issued}
                          >
                            {formatCreditAmount(a.issued)}
                          </dd>
                        </div>
                        <div className="min-w-0">
                          <dt className="text-muted-foreground">{translateUi("Used")}</dt>
                          <dd
                            className="break-words tabular-nums"
                            title={a.used}
                          >
                            {formatCreditAmount(a.used)}
                          </dd>
                        </div>
                      </dl>
                    </li>
                  ))}
                </ul>
                {!data.accounts.length && (
                  <p className="py-3 type-secondary text-muted-foreground">{translateUi("No grant issuance or usage in this window.")}</p>
                )}
              </TabsContent>
              <TabsContent value="service">
                <p className="mt-3 type-secondary text-muted-foreground">{translateUi("Grant-funded usage by service. Shared grants have no per-service granted amount.")}</p>
                <ul
                  className="divide-y divide-border/50"
                  aria-label={translateUi("Grant-funded usage by service")}
                >
                  {data.services.map((s) => (
                    <li
                      key={s.service_slug ?? ""}
                      className="flex flex-wrap justify-between gap-3 py-3 type-secondary"
                    >
                      <span className="min-w-0 break-all type-row-title">
                        {s.service_slug ??
                          translateUi("Historical usage · service unavailable")}
                      </span>
                      <span className="break-words tabular-nums" title={s.used}>
                        {formatCreditAmount(s.used)}{translateUi(" used")}</span>
                    </li>
                  ))}
                </ul>
                {!data.services.length && (
                  <p className="py-3 type-secondary text-muted-foreground">{translateUi("No grant-funded service usage on this page.")}</p>
                )}
              </TabsContent>
            </Tabs>
            {total > data.per_page && (
              <div className="flex flex-wrap items-center gap-3">
                <Button
                  variant="outline"
                  disabled={currentPage === 1 || query.isFetching}
                  onClick={() => navigate(currentPage - 1)}
                >{translateUi("Previous ")}{accountMode ? translateUi("accounts") : translateUi("services")}
                </Button>
                <span className="type-secondary text-text-tertiary">{translateUi("Page ")}{currentPage}{translateUi(" of ")}{Math.ceil(total / data.per_page)}
                </span>
                <Button
                  variant="outline"
                  disabled={
                    currentPage * data.per_page >= total || query.isFetching
                  }
                  onClick={() => navigate(currentPage + 1)}
                >{translateUi("Next ")}{accountMode ? translateUi("accounts") : translateUi("services")}
                </Button>
              </div>
            )}
          </div>
        </CardContent>
      )}
    </Card>
  );
}
function utc(value: string) {
  return new Date(value).toLocaleString(getFormattingLocale(), {
    timeZone: "UTC",
    timeZoneName: "short",
  });
}
