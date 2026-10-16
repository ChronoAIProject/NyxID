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
        <p>
          Runtime billing uses the accepted continuous-run rate and cumulative
          whole seconds. There is no per-instance cap. Paused storage is free.
        </p>
        <p>
          Gateway services have their own charges. A free Machine never makes
          downstream services free, and service costs are not added to these
          runtime totals.
        </p>
      </div>
      {billingState === "free" && (
        <p className="rounded-lg border border-border/50 p-4 text-12 text-muted-foreground">
          This resource currently reports free runtime billing. Paid historical
          contracts, if any, are shown only when the server provides them.
        </p>
      )}
      <DetailSection title="Current and historical contracts">
        <div className="space-y-3 px-4 py-3">
          <Button
            variant={selectedRunId ? "outline" : "default"}
            onClick={() => selectRun(undefined)}
          >
            View current contract
          </Button>
          {selectedRunId && (
            <p className="break-all text-12 text-muted-foreground">
              Viewing historical run {selectedRunId}. Each run retains its
              accepted rate version.
            </p>
          )}
          {runs.isPending && (
            <p role="status" className="text-12">
              Loading historical contracts…
            </p>
          )}
          {runs.error &&
            (runs.error instanceof ApiError &&
            [404, 405, 501].includes(runs.error.status) ? (
              <p className="text-12 text-muted-foreground">
                Historical contract lookup is unavailable on this server.
                Current billing remains independently available when its
                endpoint is enabled.
              </p>
            ) : (
              <ErrorBanner
                message={`Historical contracts could not be loaded. ${runs.error.message}`}
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
                  {run.accepted_credits_per_second} Credits / second ·{" "}
                  {run.state.replaceAll("_", " ")}
                </p>
                <p className="break-all text-muted-foreground">
                  Rate {run.rate_version} · Created{" "}
                  {new Date(run.created_at).toLocaleString()}
                </p>
              </div>
              <Button
                aria-label={`View run ${run.run_id}`}
                disabled={selectedRunId === run.run_id}
                onClick={() => selectRun(run.run_id)}
              >
                View contract
              </Button>
            </article>
          ))}
          {runs.data && !runs.data.runs.length && (
            <p className="text-12 text-muted-foreground">
              No runtime contracts in this page.
            </p>
          )}
          <div className="flex justify-between gap-2">
            <Button
              disabled={!runCursors.length}
              onClick={() => setRunCursors((values) => values.slice(0, -1))}
            >
              Previous contracts
            </Button>
            <Button
              disabled={!runs.data?.next_cursor || runs.isFetching}
              onClick={() => {
                const next = runs.data?.next_cursor;
                if (next) setRunCursors((values) => [...values, next]);
              }}
            >
              Next contracts
            </Button>
          </div>
        </div>
      </DetailSection>
      {query.isPending && (
        <p role="status" className="text-12">
          Loading runtime billing…
        </p>
      )}
      {query.error && (
        <div role="status" className="space-y-3">
          {missing ? (
            <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">
              Runtime billing details are unavailable on this server. Accepted
              rates, exact settled/reserved amounts, pending liability, and
              account-duration estimates cannot be shown yet.
            </p>
          ) : (
            <ErrorBanner
              message={`Runtime billing could not be loaded. ${query.error.message}`}
              onRetry={() => void query.refetch()}
            />
          )}
          <Button onClick={() => void query.refetch()}>
            Check billing availability
          </Button>
        </div>
      )}
      {data && (
        <>
          <DetailSection title="Accepted runtime contract">
            <DetailRow label="Run ID" value={data.run_id} mono copyable />
            <DetailRow
              label="Contract state"
              value={data.state.replaceAll("_", " ")}
              badge
            />
            <DetailRow label="Rate version" value={data.rate_version} mono />
            <DetailRow
              label="Accepted rate"
              value={`${data.accepted_credits_per_second} Credits / second`}
            />
            <DetailRow
              label="Cumulative billed time"
              value={`${data.cumulative_seconds} whole seconds`}
            />
            <DetailRow
              label="Funded through"
              value={new Date(data.covered_until).toLocaleString()}
            />
            <DetailRow
              label="Settled"
              value={`${data.settled_credits} Credits`}
            />
            <DetailRow
              label="Reserved"
              value={`${data.reserved_credits} Credits`}
            />
            <DetailRow
              label="Pending within reserved"
              value={`${data.pending_credits} Credits`}
            />
            <p className="px-4 py-3 text-12 text-muted-foreground">
              Pending liability is included in reserved Credits. It is not an
              additional amount to sum. Unknown stop evidence can retain
              responsibility until verified settlement; later funds do not
              retroactively cover expired execution.
            </p>
          </DetailSection>
          <DetailSection title="Shared account funds and estimate">
            <DetailRow
              label="Wallet available"
              value={`${data.funds.wallet_available} Credits`}
            />
            <DetailRow
              label="Wallet reserved across account"
              value={`${data.funds.wallet_reserved} Credits`}
            />
            <DetailRow
              label="Pending debits"
              value={`${data.funds.pending_debits} Credits`}
            />
            <DetailRow
              label="Pending purchased-credit expiry"
              value={`${data.funds.pending_expiry} Credits`}
            />
            <DetailRow
              label="Matching allowance remaining"
              value={`${data.funds.matching_allowance_seconds} seconds`}
            />
            <DetailRow
              label="Available matching grants"
              value={`${data.funds.available_grants} Credits`}
            />
            <DetailRow
              label="Current estimate"
              value={`${data.funds.estimated_seconds} seconds`}
            />
            <p className="px-4 py-3 text-12 text-muted-foreground">
              This is an account-wide estimate, not a reserved duration or
              spending limit. Other Machines and services share these funds;
              allowances and grants can expire. A finite reservation, live
              eligibility, and provider checks still govern every execution
              window.
            </p>
          </DetailSection>
          <div className="space-y-3">
            <h3 className="text-13 font-semibold">Runtime intervals</h3>
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
                    {row.settled ? "Settled" : "Reserved"}
                  </Badge>
                  <p className="text-11 text-muted-foreground">
                    Generation {row.runtime_generation} ·{" "}
                    {row.quantity === null
                      ? "Awaiting evidence"
                      : `${row.quantity} seconds`}
                  </p>
                  <p className="text-11 text-muted-foreground">
                    {row.settled ? row.settled_credits : row.reserved_credits}{" "}
                    Credits
                  </p>
                  <p className="break-all text-11 text-text-tertiary">
                    Rate {row.rate_version}
                  </p>
                </article>
              ))}
            </div>
            <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Interval</TableHead>
                    <TableHead>Generation / seconds</TableHead>
                    <TableHead>Funding</TableHead>
                    <TableHead>Status</TableHead>
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
                          {new Date(row.covered_from).toLocaleString()} –{" "}
                          {new Date(row.covered_until).toLocaleString()}
                        </span>
                        <span className="block break-all text-11 text-text-tertiary">
                          Rate {row.rate_version}
                        </span>
                      </TableCell>
                      <TableCell>
                        {row.runtime_generation} / {row.quantity ?? "Pending"}
                      </TableCell>
                      <TableCell>
                        {row.settled
                          ? row.settled_credits
                          : row.reserved_credits}{" "}
                        Credits
                        <br />
                        <span className="text-11 text-text-tertiary">
                          Allowance {row.allowance_funded} · Grant{" "}
                          {row.grant_funded} · Wallet {row.wallet_funded}
                        </span>
                      </TableCell>
                      <TableCell>
                        <Badge variant={row.settled ? "success" : "warning"}>
                          {row.settled ? "Settled" : "Reserved"}
                        </Badge>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
            {!data.intervals.length && (
              <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">
                No runtime intervals in this page.
              </p>
            )}
            <div className="flex justify-between gap-2">
              <Button
                disabled={offsets.length < 2}
                onClick={() => setOffsets((values) => values.slice(0, -1))}
              >
                Previous intervals
              </Button>
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
              >
                Next intervals
              </Button>
            </div>
          </div>
        </>
      )}
      <DetailSection title="Recovery and separate service usage">
        <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
          <p>
            Insufficient funds never triggers automatic top-up or resume. First
            resolve any old pending responsibility, then manually review a new
            quote and resume.
          </p>
          <p>
            Load Machine-attributed gateway service rows separately below. Use
            account Billing & Usage for service totals; those charges are not
            copied into Machine runtime totals.
          </p>
          <Button asChild>
            <Link to="/billing">Open Billing &amp; Usage</Link>
          </Button>
        </div>
      </DetailSection>
    </div>
  );
}
