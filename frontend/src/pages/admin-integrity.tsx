import { RefreshCw, ShieldAlert, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { PageHeader } from "@/components/shared/page-header";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useChainVerification,
  useRunChainVerification,
} from "@/hooks/use-admin-chain-verification";
import type {
  ChainVerifyStatus,
  ExternalAuditEvidence,
  StartupDiagnostic,
} from "@/schemas/admin-chain-verification";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";

const CHAIN_LABELS: Record<ChainVerifyStatus["chain"], string> = {
  audit_log: "Audit Log Chain",
  billing_ledger: "Billing Ledger Chain",
  billing_accounts: "Billing Account Balances",
};

const CHAIN_DESCRIPTIONS: Record<ChainVerifyStatus["chain"], string> = {
  billing_accounts:
    "Exact grant and wallet balances compared with their durable ledger postings in a consistent snapshot.",
  audit_log:
    "Local HMAC and link continuity of audit events against the persistent head. Independent evidence is reported separately above.",
  billing_ledger:
    "Local journal of money-moving billing events, checked against audit-chain head anchors. Independent protection of retained billing evidence is reported separately from this local check.",
};

function formatTimestamp(value: string | null | undefined): string {
  if (!value) return "never";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? "never" : parsed.toLocaleString();
}

function ChainCard({ status }: { readonly status: ChainVerifyStatus }) {
  const broken = status.outcome === "broken";
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <div className="flex items-center gap-2">
          {broken ? (
            <ShieldAlert className="h-5 w-5 text-destructive" />
          ) : (
            <ShieldCheck className="h-5 w-5 text-success" />
          )}
          <CardTitle className="text-base">
            {CHAIN_LABELS[status.chain]}
          </CardTitle>
        </div>
        <Badge variant={broken ? "destructive" : "success"}>
          {broken
            ? "Broken"
            : status.chain === "billing_accounts" && status.break_detail
              ? "Pending settlements"
              : status.chain === "billing_accounts"
                ? "Reconciled locally"
                : "Locally intact"}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          {CHAIN_DESCRIPTIONS[status.chain]}
        </p>

        {broken ? (
          <div className="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm">
            <p className="font-medium text-destructive">
              {status.chain === "billing_accounts"
                ? "Account balance mismatch"
                : `Integrity break at seq ${status.break_seq ?? "unknown"}`}
              {status.break_kind ? ` (${status.break_kind})` : ""}
            </p>
            {status.break_detail ? (
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
                {status.break_detail}
              </p>
            ) : null}
            <p className="mt-2 text-muted-foreground">
              {status.chain === "billing_accounts"
                ? "A stored balance differs from its journal. Review the account and its settlement recovery state."
                : "Stored history no longer matches its hash chain. Treat as possible tampering and investigate before trusting records at or after this seq."}
            </p>
          </div>
        ) : null}

        <dl className="grid grid-cols-2 gap-x-6 gap-y-2 text-sm sm:grid-cols-3">
          <div>
            <dt className="text-muted-foreground">Chain head</dt>
            <dd>{status.head_seq ?? "empty"}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Last check</dt>
            <dd>{formatTimestamp(status.last_run_at)}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Last full pass</dt>
            <dd>{formatTimestamp(status.last_full_pass_at)}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Entries last check</dt>
            <dd>{status.checked_entries}</dd>
          </div>
          {status.chain === "billing_ledger" ? (
            <div>
              <dt className="text-muted-foreground">Head anchor</dt>
              <dd>
                {status.anchor_seq == null
                  ? "none yet"
                  : `seq ${status.anchor_seq}${status.anchor_valid === false ? " (invalid)" : ""}`}
              </dd>
            </div>
          ) : (
            <div>
              <dt className="text-muted-foreground">Pre-chain rows</dt>
              <dd>{status.pre_chain_count ?? 0}</dd>
            </div>
          )}
        </dl>
      </CardContent>
    </Card>
  );
}

const EVIDENCE_LABELS: Record<ExternalAuditEvidence["status"], string> = {
  unwitnessed: "Unwitnessed",
  unverified: "Unverified",
  unavailable: "Unavailable",
  broken: "Conflicting history",
};

function ExternalEvidence({
  evidence,
}: {
  readonly evidence: ExternalAuditEvidence;
}) {
  const broken =
    evidence.status === "broken" ||
    evidence.diagnostics?.witness === "diverged";
  const diagnostics = evidence.diagnostics;
  const duration = (seconds: number | null) =>
    seconds === null ? "No pending deadline" : `${seconds.toLocaleString()} s`;
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <CardTitle className="text-base">Audit witness</CardTitle>
        <Badge variant={broken ? "destructive" : "warning"}>
          {diagnostics?.witness === "diverged"
            ? "Conflicting history"
            : EVIDENCE_LABELS[evidence.status]}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4 text-sm">
        <p className="text-muted-foreground">
          {evidence.status === "unwitnessed"
            ? "No independent witness is enrolled. Local checks cannot detect a coordinated database rollback."
            : evidence.status === "unverified"
              ? "A witness is enrolled, but fresh independent evidence has not been verified by this process. A stored acknowledgment alone cannot establish current continuity."
              : broken
                ? "Independent evidence conflicts with stored history. Investigate before trusting the affected records."
                : "Independent evidence is unavailable. Local verification cannot establish protection against database rollback."}
        </p>
        {diagnostics ? (
          <div className="space-y-4">
            <p className="text-muted-foreground">
              Witness state:{" "}
              <span
                className={
                  broken ? "font-medium text-destructive" : "font-medium"
                }
              >
                {diagnostics.witness}
              </span>
              {diagnostics.last_contact_secs === null
                ? " · Contact freshness unknown since process start"
                : ` · Last authenticated contact ${duration(diagnostics.last_contact_secs)} ago`}
            </p>
            {diagnostics.last_transition ? (
              <p className="text-muted-foreground">
                Last transition: {diagnostics.last_transition.reason} at{" "}
                {formatTimestamp(diagnostics.last_transition.at)}
              </p>
            ) : null}
            <dl className="grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-3">
              <div>
                <dt className="text-muted-foreground">Chain identity</dt>
                <dd className="break-all font-mono text-xs">
                  {diagnostics.chain_id ?? "unknown"}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Oldest pending age</dt>
                <dd>
                  {diagnostics.oldest_pending_secs === null
                    ? "unknown"
                    : duration(diagnostics.oldest_pending_secs)}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">
                  Standard time to closure
                </dt>
                <dd>{duration(diagnostics.standard_closes_in_secs)}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">
                  Availability time to closure
                </dt>
                <dd>{duration(diagnostics.availability_closes_in_secs)}</dd>
              </div>
            </dl>
            <div
              className="grid grid-cols-2 gap-2 sm:grid-cols-4"
              aria-label="Audit admission by tier"
            >
              {Object.entries(diagnostics.admission).map(
                ([tier, admission]) => (
                  <div key={tier} className="rounded-md border p-2">
                    <p className="capitalize">{tier}</p>
                    <Badge
                      variant={
                        admission === "closed"
                          ? "destructive"
                          : admission === "alarm"
                            ? "warning"
                            : "secondary"
                      }
                    >
                      {admission}
                    </Badge>
                  </div>
                ),
              )}
            </div>
            {diagnostics.divergence ? (
              <div className="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-destructive">
                <p>
                  Independent history diverged at sequence{" "}
                  {diagnostics.divergence.seq}.
                </p>
                <p className="font-mono text-xs">
                  Local {diagnostics.divergence.local_hash_prefix} · Witness{" "}
                  {diagnostics.divergence.witnessed_hash_prefix}
                </p>
              </div>
            ) : null}
            <details className="text-muted-foreground">
              <summary className="cursor-pointer">
                Effective audit limits
              </summary>
              <p className="mt-2">
                Soft alarms: contact {duration(diagnostics.limits.contact_secs)}
                , oldest pending {duration(diagnostics.limits.pending_age_secs)}
                , {diagnostics.limits.pending_count.toLocaleString()} pending
                events.
              </p>
              <p>
                Standard ceiling:{" "}
                {duration(diagnostics.limits.standard.pending_age_secs)} or{" "}
                {diagnostics.limits.standard.pending_count.toLocaleString()}{" "}
                events.
              </p>
              <p>
                Availability ceiling:{" "}
                {duration(diagnostics.limits.availability.pending_age_secs)} or{" "}
                {diagnostics.limits.availability.pending_count.toLocaleString()}{" "}
                events.
              </p>
            </details>
          </div>
        ) : null}
        <a
          className="text-primary underline"
          href="https://github.com/ChronoAIProject/NyxID/blob/main/docs/SECURITY_AUDIT_WITNESS.md#admin-custody-replacement-and-dr-restore"
          target="_blank"
          rel="noreferrer"
        >
          Witness recovery runbook
        </a>
        <dl className="grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-3">
          <div>
            <dt className="text-muted-foreground">Local audit head</dt>
            <dd>{evidence.local_head_seq ?? "unknown"}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Stored witness cursor</dt>
            <dd>{evidence.witnessed_seq ?? "none"}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Pending evidence records</dt>
            <dd>{evidence.pending_count ?? "unknown"}</dd>
          </div>
          {evidence.oldest_pending_at ? (
            <div>
              <dt className="text-muted-foreground">Oldest pending record</dt>
              <dd>{formatTimestamp(evidence.oldest_pending_at)}</dd>
            </div>
          ) : null}
        </dl>
      </CardContent>
    </Card>
  );
}

function StartupDiagnostics({
  diagnostics,
}: {
  readonly diagnostics: readonly StartupDiagnostic[];
}) {
  if (diagnostics.length === 0) return null;

  return (
    <Card className="border-destructive/50">
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <div className="flex items-center gap-2">
          <ShieldAlert className="h-5 w-5 text-destructive" />
          <CardTitle className="text-base">Startup diagnostics</CardTitle>
        </div>
        <Badge variant="destructive">Action required</Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        {diagnostics.map((diagnostic) => (
          <div key={diagnostic.code} className="space-y-2 text-sm">
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <p className="font-medium text-destructive">
                {diagnostic.summary}
              </p>
              <span className="text-xs text-muted-foreground">
                {formatTimestamp(diagnostic.detected_at)}
              </span>
            </div>
            <p className="text-muted-foreground">{diagnostic.detail}</p>
            <p>{diagnostic.remediation}</p>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

export function AdminIntegrityPage() {
  const { data, isLoading, error } = useChainVerification();
  const runVerification = useRunChainVerification();
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);

  const handleRun = () => {
    runVerification.mutate(undefined, {
      onSuccess: (result) => {
        const broken = result.chains.filter(
          (chain) => chain.outcome === "broken",
        );
        if (result.external_evidence.status === "broken") {
          toast.error("Independent evidence conflicts with stored history");
        } else if (broken.length === 0) {
          toast.warning(
            `Local check completed; independent evidence: ${EVIDENCE_LABELS[result.external_evidence.status].toLowerCase()}`,
          );
        } else {
          toast.error(
            `Integrity break detected in ${broken
              .map((chain) => CHAIN_LABELS[chain.chain])
              .join(", ")}`,
          );
        }
      },
      onError: () => toast.error("Verification run failed"),
    });
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Integrity"
        description="Tamper-evidence status of the audit log and billing ledger hash chains, re-verified automatically in the background."
        actions={
          canWrite ? (
            <Button
              onClick={handleRun}
              disabled={runVerification.isPending}
              variant="outline"
            >
              <RefreshCw
                className={
                  runVerification.isPending
                    ? "mr-2 h-4 w-4 animate-spin"
                    : "mr-2 h-4 w-4"
                }
              />
              Run check now
            </Button>
          ) : undefined
        }
      />

      {data ? (
        <>
          <ExternalEvidence evidence={data.external_evidence} />
          <StartupDiagnostics diagnostics={data.startup_diagnostics} />
        </>
      ) : null}

      {error ? (
        <Card>
          <CardContent className="py-8 text-center text-sm text-muted-foreground">
            Failed to load chain verification status.
          </CardContent>
        </Card>
      ) : isLoading ? (
        <div className="space-y-4">
          <Skeleton className="h-48 w-full" />
          <Skeleton className="h-48 w-full" />
        </div>
      ) : data && data.chains.length > 0 ? (
        <div className="space-y-4">
          {[...data.chains, ...(data.accounts ? [data.accounts] : [])].map(
            (status) => (
              <ChainCard key={status.chain} status={status} />
            ),
          )}
        </div>
      ) : (
        <Card>
          <CardContent className="py-8 text-center text-sm text-muted-foreground">
            No verification runs recorded yet. The background sweep runs on an
            interval after startup; use Run check now to verify immediately.
          </CardContent>
        </Card>
      )}
    </div>
  );
}
