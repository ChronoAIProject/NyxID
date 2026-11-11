import { translateErrorMessage } from "@/lib/error-messages";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
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
  useUiTranslation();
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
  // The mounted API has two explicit missing-contract responses: the owner
  // handler returns 404 before a run exists; the detail service returns 400
  // for an absent selected run. AppError prefixes its wire messages; retain
  // compatibility with unprefixed responses without treating generic 404s as empty.
  const noContract =
    query.error instanceof ApiError &&
    ((query.error.status === 404 &&
      [
        "runtime billing contract not found",
        "Not found: runtime billing contract not found",
      ].includes(translateErrorMessage(query.error.message))) ||
      (query.error.status === 400 &&
        query.error.errorCode === 1008 &&
        [
          "runtime contract not found",
          "Validation error: runtime contract not found",
        ].includes(translateErrorMessage(query.error.message))));
  const missing =
    query.error instanceof ApiError &&
    [404, 405, 501].includes(query.error.status);
  return (
    <div className="space-y-6">
      <div className="space-y-2 text-14 text-muted-foreground">
        <p>{translateUi("Runtime billing uses the accepted continuous-run rate and cumulative whole seconds. There is no per-instance cap. Paused storage is free.")}</p>
        <p>{translateUi("Gateway services have their own charges. A free Machine never makes downstream services free, and service costs are not added to these runtime totals.")}</p>
      </div>
      {billingState === "free" && (
        <p className="rounded-lg border border-border/50 p-4 text-14 text-muted-foreground">{translateUi("This resource currently reports free runtime billing. Paid historical contracts, if any, are shown only when the server provides them.")}</p>
      )}
      <DetailSection title={translateUi("Current and historical contracts")}>
        <div className="space-y-3 px-4 py-3">
          <Button
            variant={selectedRunId ? "outline" : "default"}
            onClick={() => selectRun(undefined)}
          >{translateUi("View current contract")}</Button>
          {selectedRunId && (
            <p className="break-all text-14 text-muted-foreground">{translateUi("Viewing historical run ")}{selectedRunId}. Each run retains its
              accepted rate version.
            </p>
          )}
          <p className="text-14 text-muted-foreground">{translateUi("Historical summaries contain accepted rates and state. Open a contract to read its settled and reserved amounts separately.")}</p>
          {runs.isPending && (
            <p role="status" className="text-14">{translateUi("Loading historical contracts…")}</p>
          )}
          {runs.error &&
            (runs.error instanceof ApiError &&
            [404, 405, 501].includes(runs.error.status) ? (
              <p className="text-14 text-muted-foreground">{translateUi("Historical contract lookup is unavailable on this server. Current billing remains independently available when its endpoint is enabled.")}</p>
            ) : (
              <ErrorBanner
                message={translateUi("Historical contracts could not be loaded. {{value1}}", { value1: runs.error.message })}
                onRetry={() => void runs.refetch()}
              />
            ))}
          {runs.data?.runs.map((run) => (
            <article
              key={run.run_id}
              className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border/50 p-3"
            >
              <div className="min-w-0 space-y-1 text-14">
                <p className="break-all font-mono">{run.run_id}</p>
                <p>
                  {run.accepted_credits_per_second}{translateUi(" Credits / second ·")}{" "}
                  {run.state.replaceAll("_", " ")}
                </p>
                <p className="break-all text-muted-foreground">{translateUi("Rate ")}{run.rate_version}{translateUi(" · Created")}{" "}
                  {new Date(run.created_at).toLocaleString(getFormattingLocale())}
                </p>
              </div>
              <Button
                aria-label={translateUi("View run {{value1}}", { value1: run.run_id })}
                disabled={selectedRunId === run.run_id}
                onClick={() => selectRun(run.run_id)}
              >{translateUi("View contract")}</Button>
            </article>
          ))}
          {runs.data && !runs.data.runs.length && (
            <p className="text-14 text-muted-foreground">{translateUi("No runtime contracts in this page.")}</p>
          )}
          <div className="flex justify-between gap-2">
            <Button
              disabled={!runCursors.length}
              onClick={() => setRunCursors((values) => values.slice(0, -1))}
            >{translateUi("Previous contracts")}</Button>
            <Button
              disabled={!runs.data?.next_cursor || runs.isFetching}
              onClick={() => {
                const next = runs.data?.next_cursor;
                if (next) setRunCursors((values) => [...values, next]);
              }}
            >{translateUi("Next contracts")}</Button>
          </div>
        </div>
      </DetailSection>
      {query.isPending && (
        <p role="status" className="text-14">{translateUi("Loading runtime billing…")}</p>
      )}
      {query.error && (
        <div role="status" className="space-y-3">
          {noContract ? (
            <p className="rounded-lg border border-dashed border-border p-4 text-14 text-muted-foreground">
              {selectedRunId
                ? translateUi("Selected historical contract was not found. Refresh the contract list or view the current contract; its amounts remain unknown.")
                : billingState === "free"
                  ? translateUi("No current paid runtime contract. This is expected for free runtime; downstream service usage remains separate. Historical contracts can still be reviewed above.")
                  : translateUi("No current runtime contract was found. Current billing amounts remain unknown; this does not establish zero cost or completed settlement.")}
            </p>
          ) : missing ? (
            <p className="rounded-lg border border-dashed border-border p-4 text-14 text-muted-foreground">{translateUi("Runtime billing details are unavailable on this server. Accepted rates, exact settled/reserved amounts, pending liability, and account-duration estimates cannot be shown yet.")}</p>
          ) : (
            <ErrorBanner
              message={translateUi("Runtime billing could not be loaded. {{value1}}", { value1: query.error.message })}
              onRetry={() => void query.refetch()}
            />
          )}
          <Button
            onClick={() => {
              void query.refetch();
              if (noContract) void runs.refetch();
            }}
          >
            {noContract ? translateUi("Refresh contracts") : translateUi("Check billing availability")}
          </Button>
        </div>
      )}
      {data && (
        <>
          <DetailSection
            title={
              selectedRunId
                ? translateUi("Historical runtime contract")
                : translateUi("Current runtime contract")
            }
          >
            <DetailRow label={translateUi("Run ID")} value={data.run_id} mono copyable />
            <DetailRow
              label={translateUi("Contract state")}
              value={data.state.replaceAll("_", " ")}
              badge
            />
            <DetailRow label={translateUi("Rate version")} value={data.rate_version} mono />
            <DetailRow
              label={translateUi("Accepted rate")}
              value={`${data.accepted_credits_per_second} Credits / second`}
            />
            <DetailRow
              label={translateUi("Cumulative billed time")}
              value={`${data.cumulative_seconds} whole seconds`}
            />
            <DetailRow
              label={translateUi("Funded through")}
              value={new Date(data.covered_until).toLocaleString(getFormattingLocale())}
            />
            <DetailRow
              label={translateUi("Settled")}
              value={`${data.settled_credits} Credits`}
            />
            <DetailRow
              label={translateUi("Reserved (includes pending)")}
              value={`${data.reserved_credits} Credits`}
            />
            <DetailRow
              label={translateUi("Pending within reserved")}
              value={`${data.pending_credits} Credits`}
            />
            <p className="px-4 py-3 text-14 text-muted-foreground">{translateUi("Pending liability is included in reserved Credits. It is not an additional amount to sum. Unknown stop evidence can retain responsibility until verified settlement; later funds do not retroactively cover expired execution.")}</p>
          </DetailSection>
          <DetailSection title={translateUi("Shared account funds and estimate")}>
            <DetailRow
              label={translateUi("Wallet available")}
              value={`${data.funds.wallet_available} Credits`}
            />
            <DetailRow
              label={translateUi("Wallet reserved across account")}
              value={`${data.funds.wallet_reserved} Credits`}
            />
            <DetailRow
              label={translateUi("Pending debits")}
              value={`${data.funds.pending_debits} Credits`}
            />
            <DetailRow
              label={translateUi("Pending purchased-credit expiry")}
              value={`${data.funds.pending_expiry} Credits`}
            />
            <DetailRow
              label={translateUi("Matching allowance remaining")}
              value={`${data.funds.matching_allowance_seconds} seconds`}
            />
            <DetailRow
              label={translateUi("Available matching grants")}
              value={`${data.funds.available_grants} Credits`}
            />
            <DetailRow
              label={translateUi("Current estimate")}
              value={`${data.funds.estimated_seconds} seconds`}
            />
            <p className="px-4 py-3 text-14 text-muted-foreground">{translateUi("This is an account-wide estimate, not a reserved duration or spending limit. Other Machines and services share these funds; allowances and grants can expire. A finite reservation, live eligibility, and provider checks still govern every execution window.")}</p>
          </DetailSection>
          <div className="space-y-3">
            <h3 className="type-card-title">{translateUi("Runtime intervals")}</h3>
            <div className="flex flex-col gap-3 md:hidden">
              {data.intervals.map((row) => (
                <article
                  key={row.interval_id}
                  className="space-y-2 rounded-xl border border-border/50 bg-card p-4"
                >
                  <p className="break-all type-card-title">
                    {row.interval_id}
                  </p>
                  <Badge variant={row.settled ? "success" : "warning"}>
                    {row.settled ? translateUi("Settled") : translateUi("Reserved")}
                  </Badge>
                  <p className="text-13 text-muted-foreground">{translateUi("Generation ")}{row.runtime_generation} ·{" "}
                    {row.quantity === null
                      ? translateUi("Awaiting evidence")
                      : translateUi("{{value1}} seconds", { value1: row.quantity })}
                  </p>
                  <p className="text-13 text-muted-foreground">
                    {row.settled ? row.settled_credits : row.reserved_credits}{" "}{translateUi("Credits")}</p>
                  <p className="break-all text-13 text-text-tertiary">{translateUi("Rate ")}{row.rate_version}
                  </p>
                </article>
              ))}
            </div>
            <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{translateUi("Interval")}</TableHead>
                    <TableHead>{translateUi("Generation / seconds")}</TableHead>
                    <TableHead>{translateUi("Funding")}</TableHead>
                    <TableHead>{translateUi("Status")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {data.intervals.map((row) => (
                    <TableRow key={row.interval_id}>
                      <TableCell>
                        <span className="block break-all font-mono text-13">
                          {row.interval_id}
                        </span>
                        <span className="text-13 text-text-tertiary">
                          {new Date(row.covered_from).toLocaleString(getFormattingLocale())} –{" "}
                          {new Date(row.covered_until).toLocaleString(getFormattingLocale())}
                        </span>
                        <span className="block break-all text-13 text-text-tertiary">{translateUi("Rate ")}{row.rate_version}
                        </span>
                      </TableCell>
                      <TableCell>
                        {row.runtime_generation} / {row.quantity ?? translateUi("Pending")}
                      </TableCell>
                      <TableCell>
                        {row.settled
                          ? row.settled_credits
                          : row.reserved_credits}{" "}{translateUi("Credits")}<br />
                        <span className="text-13 text-text-tertiary">{translateUi("Allowance ")}{row.allowance_funded}{translateUi(" · Grant")}{" "}
                          {row.grant_funded}{translateUi(" · Wallet ")}{row.wallet_funded}
                        </span>
                      </TableCell>
                      <TableCell>
                        <Badge variant={row.settled ? "success" : "warning"}>
                          {row.settled ? translateUi("Settled") : translateUi("Reserved")}
                        </Badge>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
            {!data.intervals.length && (
              <p className="rounded-lg border border-dashed border-border p-4 text-14 text-muted-foreground">{translateUi("No runtime intervals in this page.")}</p>
            )}
            <div className="flex justify-between gap-2">
              <Button
                disabled={offsets.length < 2}
                onClick={() => setOffsets((values) => values.slice(0, -1))}
              >{translateUi("Previous intervals")}</Button>
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
              >{translateUi("Next intervals")}</Button>
            </div>
          </div>
        </>
      )}
      <DetailSection title={translateUi("Recovery and separate service usage")}>
        <div className="space-y-3 px-4 py-3 text-14 text-muted-foreground">
          <p>{translateUi("Insufficient funds never triggers automatic top-up or resume. First resolve any old pending responsibility, then manually review a new quote and resume.")}</p>
          <p>{translateUi("Load Machine-attributed gateway service rows separately below. Use account Billing & Usage for service totals; those charges are not copied into Machine runtime totals.")}</p>
          <Button asChild>
            <Link to="/billing">{translateUi("Open Billing & Usage")}</Link>
          </Button>
        </div>
      </DetailSection>
    </div>
  );
}
