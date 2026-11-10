import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateErrorMessage } from "@/lib/error-messages";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  useRemoteMachineBillingRuns,
  useRemoteMachineRuntimeBilling,
} from "@/hooks/use-machines";
import { ApiError } from "@/lib/api-client";
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export function RemoteMachineBilling({
  machineId,
  billingState,
  runId,
}: {
  readonly machineId: string;
  readonly billingState?: string;
  readonly runId?: string;
}) {
  const { t } = useUiTranslation();
  const [offsets, setOffsets] = useState<number[]>([0]);
  const [selectedRunId, setSelectedRunId] = useState(runId);
  const [runCursors, setRunCursors] = useState<string[]>([]);
  const runs = useRemoteMachineBillingRuns(machineId, runCursors.at(-1));
  const selectRun = (nextRunId: string | undefined) => {
    setSelectedRunId(nextRunId);
    setOffsets([0]);
  };
  const query = useRemoteMachineRuntimeBilling(
    machineId,
    selectedRunId,
    offsets.at(-1) ?? 0,
  );
  const data = query.data;
  const missing =
    query.error instanceof ApiError &&
    [404, 405, 501].includes(query.error.status);
  return (
    <div className="space-y-6">
      <div className="space-y-2 text-12 text-muted-foreground">
        <p>{t("Runtime billing uses the accepted continuous-run rate and cumulative whole seconds. There is no per-instance cap. Paused storage is free.")}</p>
        <p>{t("Gateway services have their own charges. A free Machine never makes downstream services free, and service costs are not added to these runtime totals.")}</p>
      </div>
      {billingState === "free" && (
        <p className="rounded-lg border border-border/50 p-4 text-12 text-muted-foreground">{t("This resource currently reports free runtime billing. Paid historical contracts, if any, are shown only when the server provides them.")}</p>
      )}
      <DetailSection title={t("Current and historical contracts")}>
        <div className="space-y-3 px-4 py-3">
          <Button
            variant={selectedRunId ? "outline" : "default"}
            onClick={() => selectRun(undefined)}
          >{t("View current contract")}</Button>
          {selectedRunId && (
            <p className="break-all text-12 text-muted-foreground">{t("Viewing historical run ")}{selectedRunId}{t(". Each run retains its accepted rate version.")}</p>
          )}
          {runs.isPending && (
            <p role="status" className="text-12">{t("Loading historical contracts…")}</p>
          )}
          {runs.error &&
            (runs.error instanceof ApiError &&
            [404, 405, 501].includes(runs.error.status) ? (
              <p className="text-12 text-muted-foreground">{t("Historical contract lookup is unavailable on this server. Current billing remains independently available when its endpoint is enabled.")}</p>
            ) : (
              <ErrorBanner
                message={t("Historical contracts could not be loaded. {{message}}", { message: translateErrorMessage(runs.error.message) })}
                onRetry={() => void runs.refetch()}
              />
            ))}
          {runs.data?.runs.map((run) => (
            <article
              key={run.run_id}
              className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border/50 p-3"
            >
              <div className="min-w-0 space-y-1 text-12">
                <p className="break-all font-mono">{run.run_id}</p>
                <p>
                  {run.accepted_credits_per_second}{t(" Credits / second ·")}{" "}
                  {run.state.replaceAll("_", " ")}
                </p>
                <p className="break-all text-muted-foreground">{t("Rate ")}{run.rate_version}{t(" · Created")}{" "}
                  {new Date(run.created_at).toLocaleString(getFormattingLocale())}
                </p>
              </div>
              <Button
                aria-label={t("View run {{run_id}}", { run_id: run.run_id })}
                disabled={selectedRunId === run.run_id}
                onClick={() => selectRun(run.run_id)}
              >{t("View contract")}</Button>
            </article>
          ))}
          {runs.data && !runs.data.runs.length && (
            <p className="text-12 text-muted-foreground">{t("No runtime contracts in this page.")}</p>
          )}
          <div className="flex justify-between gap-2">
            <Button
              disabled={!runCursors.length}
              onClick={() => setRunCursors((values) => values.slice(0, -1))}
            >{t("Previous contracts")}</Button>
            <Button
              disabled={!runs.data?.next_cursor || runs.isFetching}
              onClick={() => {
                const next = runs.data?.next_cursor;
                if (next) setRunCursors((values) => [...values, next]);
              }}
            >{t("Next contracts")}</Button>
          </div>
        </div>
      </DetailSection>
      {query.isPending && (
        <p role="status" className="text-12">{t("Loading runtime billing…")}</p>
      )}
      {query.error && (
        <div role="status" className="space-y-3">
          {missing ? (
            <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">{t("Runtime billing details are unavailable on this server. Accepted rates, exact settled/reserved amounts, pending liability, and account-duration estimates cannot be shown yet.")}</p>
          ) : (
            <ErrorBanner
              message={t("Runtime billing could not be loaded. {{message}}", { message: translateErrorMessage(query.error.message) })}
              onRetry={() => void query.refetch()}
            />
          )}
          <Button onClick={() => void query.refetch()}>{t("Check billing availability")}</Button>
        </div>
      )}
      {data && (
        <>
          <DetailSection title={t("Accepted runtime contract")}>
            <DetailRow label={t("Run ID")} value={data.run_id} mono copyable />
            <DetailRow
              label={t("Contract state")}
              value={data.state.replaceAll("_", " ")}
              badge
            />
            <DetailRow label={t("Rate version")} value={data.rate_version} mono />
            <DetailRow
              label={t("Accepted rate")}
              value={`${data.accepted_credits_per_second} Credits / second`}
            />
            <DetailRow
              label={t("Cumulative billed time")}
              value={`${data.cumulative_seconds} whole seconds`}
            />
            <DetailRow
              label={t("Funded through")}
              value={new Date(data.covered_until).toLocaleString(getFormattingLocale())}
            />
            <DetailRow
              label={t("Settled")}
              value={`${data.settled_credits} Credits`}
            />
            <DetailRow
              label={t("Reserved")}
              value={`${data.reserved_credits} Credits`}
            />
            <DetailRow
              label={t("Pending within reserved")}
              value={`${data.pending_credits} Credits`}
            />
            <p className="px-4 py-3 text-12 text-muted-foreground">{t("Pending liability is included in reserved Credits. It is not an additional amount to sum. Unknown stop evidence can retain responsibility until verified settlement; later funds do not retroactively cover expired execution.")}</p>
          </DetailSection>
          <DetailSection title={t("Shared account funds and estimate")}>
            <DetailRow
              label={t("Wallet available")}
              value={`${data.funds.wallet_available} Credits`}
            />
            <DetailRow
              label={t("Wallet reserved across account")}
              value={`${data.funds.wallet_reserved} Credits`}
            />
            <DetailRow
              label={t("Pending debits")}
              value={`${data.funds.pending_debits} Credits`}
            />
            <DetailRow
              label={t("Pending purchased-credit expiry")}
              value={`${data.funds.pending_expiry} Credits`}
            />
            <DetailRow
              label={t("Matching allowance remaining")}
              value={`${data.funds.matching_allowance_seconds} seconds`}
            />
            <DetailRow
              label={t("Available matching grants")}
              value={`${data.funds.available_grants} Credits`}
            />
            <DetailRow
              label={t("Current estimate")}
              value={`${data.funds.estimated_seconds} seconds`}
            />
            <p className="px-4 py-3 text-12 text-muted-foreground">{t("This is an account-wide estimate, not a reserved duration or spending limit. Other Machines and services share these funds; allowances and grants can expire. A finite reservation, live eligibility, and provider checks still govern every execution window.")}</p>
          </DetailSection>
          <div className="space-y-3">
            <h3 className="text-13 font-semibold">{t("Runtime intervals")}</h3>
            <div className="flex flex-col gap-3 md:hidden">
              {data.intervals.map((row) => (
                <article
                  key={row.interval_id}
                  className="space-y-2 rounded-xl border border-border/50 bg-card p-4"
                >
                  <p className="break-all text-13 font-semibold">
                    {row.interval_id}
                  </p>
                  <Badge variant={row.settled ? "success" : "warning"}>
                    {row.settled ? t("Settled") : t("Reserved")}
                  </Badge>
                  <p className="text-11 text-muted-foreground">{t("Generation ")}{row.runtime_generation} ·{" "}
                    {row.quantity === null
                      ? t("Awaiting evidence")
                      : t("{{quantity}} seconds", { quantity: row.quantity })}
                  </p>
                  <p className="text-11 text-muted-foreground">
                    {row.settled ? row.settled_credits : row.reserved_credits}{" "}{t("Credits")}</p>
                  <p className="break-all text-11 text-text-tertiary">{t("Rate ")}{row.rate_version}
                  </p>
                </article>
              ))}
            </div>
            <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("Interval")}</TableHead>
                    <TableHead>{t("Generation / seconds")}</TableHead>
                    <TableHead>{t("Funding")}</TableHead>
                    <TableHead>{t("Status")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {data.intervals.map((row) => (
                    <TableRow key={row.interval_id}>
                      <TableCell>
                        <span className="block break-all font-mono text-11">
                          {row.interval_id}
                        </span>
                        <span className="text-11 text-text-tertiary">
                          {new Date(row.covered_from).toLocaleString(getFormattingLocale())} –{" "}
                          {new Date(row.covered_until).toLocaleString(getFormattingLocale())}
                        </span>
                        <span className="block break-all text-11 text-text-tertiary">{t("Rate ")}{row.rate_version}
                        </span>
                      </TableCell>
                      <TableCell>
                        {row.runtime_generation} / {row.quantity ?? t("Pending")}
                      </TableCell>
                      <TableCell>
                        {row.settled
                          ? row.settled_credits
                          : row.reserved_credits}{" "}{t("Credits")}<br />
                        <span className="text-11 text-text-tertiary">{t("Allowance ")}{row.allowance_funded}{t(" · Grant")}{" "}
                          {row.grant_funded}{t(" · Wallet ")}{row.wallet_funded}
                        </span>
                      </TableCell>
                      <TableCell>
                        <Badge variant={row.settled ? "success" : "warning"}>
                          {row.settled ? t("Settled") : t("Reserved")}
                        </Badge>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
            {!data.intervals.length && (
              <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">{t("No runtime intervals in this page.")}</p>
            )}
            <div className="flex justify-between gap-2">
              <Button
                disabled={offsets.length < 2}
                onClick={() => setOffsets((values) => values.slice(0, -1))}
              >{t("Previous intervals")}</Button>
              <Button
                disabled={
                  data.next_interval_offset === null || query.isFetching
                }
                onClick={() => {
                  if (data.next_interval_offset !== null)
                    setOffsets((values) => [
                      ...values,
                      data.next_interval_offset!,
                    ]);
                }}
              >{t("Next intervals")}</Button>
            </div>
          </div>
        </>
      )}
      <DetailSection title={t("Recovery and separate service usage")}>
        <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
          <p>{t("Insufficient funds never triggers automatic top-up or resume. First resolve any old pending responsibility, then manually review a new quote and resume.")}</p>
          <p>{t("Load Machine-attributed gateway service rows separately below. Use account Billing & Usage for service totals; those charges are not copied into Machine runtime totals.")}</p>
          <Button asChild>
            <Link to="/billing">{t("Open Billing & Usage")}</Link>
          </Button>
        </div>
      </DetailSection>
    </div>
  );
}
