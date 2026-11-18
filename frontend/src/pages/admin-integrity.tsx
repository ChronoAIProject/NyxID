import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { getFormattingLocale } from "@/lib/formatting-locale";
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
  SecurityEffectDiagnostics,
  StartupDiagnostic,
} from "@/schemas/admin-chain-verification";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";

const CHAIN_LABELS: Record<ChainVerifyStatus["chain"], string> = {
  get audit_log() { return translateUi("Audit Log Chain"); },
  get billing_ledger() { return translateUi("Billing Ledger Chain"); },
  get billing_accounts() { return translateUi("Billing Account Balances"); },
};

const CHAIN_DESCRIPTIONS: Record<ChainVerifyStatus["chain"], string> = {
  billing_accounts:
    "Exact grant and wallet balances compared with their durable ledger postings in a consistent snapshot.",
  audit_log:
    "Hash-chained audit events. A break means a stored audit row was edited, deleted, or reordered after it was written.",
  billing_ledger:
    "Append-only journal of money-moving billing events, with its head anchored into the audit chain so tail truncation is detectable.",
};

function formatTimestamp(value: string | null | undefined): string {
  if (!value) return translateUi("never");
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? translateUi("never") : parsed.toLocaleString(getFormattingLocale());
}

function ChainCard({ status }: { readonly status: ChainVerifyStatus }) {
  useUiTranslation();
  const broken = status.outcome === "broken";
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <div className="flex items-center gap-2">
          {broken ? (
            <ShieldAlert className="h-5 w-5 text-destructive" />
          ) : (
            <ShieldCheck className="h-5 w-5 text-emerald-500" />
          )}
          <CardTitle className="text-15">
            {CHAIN_LABELS[status.chain]}
          </CardTitle>
        </div>
        <Badge variant={broken ? "destructive" : "success"}>
          {broken
            ? translateUi("Broken")
            : status.chain === "billing_accounts" && status.break_detail
              ? translateUi("Pending settlements")
              : translateUi("Intact")}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-14 text-muted-foreground">
          {CHAIN_DESCRIPTIONS[status.chain]}
        </p>

        {broken ? (
          <div className="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-14">
            <p className="font-medium text-destructive">
              {status.chain === "billing_accounts"
                ? translateUi("Account balance mismatch")
                : translateUi("Integrity break at seq {{value1}}", { value1: status.break_seq ?? "unknown" })}
              {status.break_kind ? translateUi(" ({{break_kind}})", { break_kind: status.break_kind }) : ""}
            </p>
            {status.break_detail ? (
              <p className="mt-1 break-all font-mono text-13 text-muted-foreground">
                {status.break_detail}
              </p>
            ) : null}
            <p className="mt-2 text-muted-foreground">
              {status.chain === "billing_accounts"
                ? translateUi("A stored balance differs from its journal. Review the account and its settlement recovery state.")
                : translateUi("Stored history no longer matches its hash chain. Treat as possible tampering and investigate before trusting records at or after this seq.")}
            </p>
          </div>
        ) : null}

        <dl className="grid grid-cols-2 gap-x-6 gap-y-2 text-14 sm:grid-cols-3">
          <div>
            <dt className="text-muted-foreground">{translateUi("Chain head")}</dt>
            <dd>{status.head_seq ?? translateUi("empty")}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">{translateUi("Last check")}</dt>
            <dd>{formatTimestamp(status.last_run_at)}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">{translateUi("Last full pass")}</dt>
            <dd>{formatTimestamp(status.last_full_pass_at)}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">{translateUi("Entries last check")}</dt>
            <dd>{status.checked_entries}</dd>
          </div>
          {status.chain === "billing_ledger" ? (
            <div>
              <dt className="text-muted-foreground">{translateUi("Head anchor")}</dt>
              <dd>
                {status.anchor_seq == null
                  ? translateUi("none yet")
                  : translateUi("seq {{value1}}{{value2}}", { value1: status.anchor_seq, value2: status.anchor_valid === false ? translateUi(" (invalid)") : "" })}
              </dd>
            </div>
          ) : (
            <div>
              <dt className="text-muted-foreground">{translateUi("Pre-chain rows")}</dt>
              <dd>{status.pre_chain_count ?? 0}</dd>
            </div>
          )}
        </dl>
      </CardContent>
    </Card>
  );
}

function DiagnosticEntry({
  diagnostic,
  action = false,
}: {
  readonly diagnostic: StartupDiagnostic;
  readonly action?: boolean;
}) {
  useUiTranslation();
  return (
    <div className={action ? "space-y-2 text-14" : "space-y-2 text-14 text-muted-foreground"}>
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <p className={action ? "font-medium text-destructive" : "font-medium"}>
          {diagnostic.summary}
        </p>
        <span className="text-13 text-muted-foreground">
          {formatTimestamp(diagnostic.detected_at)}
        </span>
      </div>
      <p className="text-muted-foreground">{diagnostic.detail}</p>
      <p>{diagnostic.remediation}</p>
      {diagnostic.technical_details ? (
        <details>
          <summary className="cursor-pointer">{translateUi("Details")}</summary>
          <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-lg border border-border bg-muted px-4 py-3 font-mono type-code">
            {diagnostic.technical_details}
          </pre>
        </details>
      ) : null}
    </div>
  );
}

function StartupDiagnostics({
  diagnostics,
}: {
  readonly diagnostics: readonly StartupDiagnostic[];
}) {
  useUiTranslation();
  if (diagnostics.length === 0) return null;

  const informationalCodes = ["billing_usage_warming", "billing_usage_coverage"];
  const needsAction = (item: StartupDiagnostic) =>
    item.severity ? item.severity === "action" : !informationalCodes.includes(item.code);
  const informational = diagnostics.filter((item) => !needsAction(item));
  const failures = diagnostics.filter(needsAction);
  return (
    <>
      {informational.map((diagnostic) => informationalCodes.includes(diagnostic.code) ? (
        <div key={diagnostic.code} role="status" className="space-y-1 text-14 text-muted-foreground">
          <p>{diagnostic.summary}{diagnostic.code === "billing_usage_warming" ? <> · {translateUi("Since {{value1}}", {
            value1: formatTimestamp(diagnostic.detected_at),
          })}</> : null}</p>
          <p>{diagnostic.detail}</p>
        </div>
      ) : <DiagnosticEntry key={diagnostic.code} diagnostic={diagnostic} />)}
      {failures.length > 0 ? (
        <Card className="border-destructive/50">
          <CardHeader className="flex flex-row items-center justify-between space-y-0">
            <div className="flex items-center gap-2">
              <ShieldAlert className="h-5 w-5 text-destructive" />
              <CardTitle className="text-15">{translateUi("Startup diagnostics")}</CardTitle>
            </div>
            <Badge variant="destructive">{translateUi("Action required")}</Badge>
          </CardHeader>
          <CardContent className="space-y-4">
            {failures.map((diagnostic) => (
              <DiagnosticEntry key={diagnostic.code} diagnostic={diagnostic} action />
            ))}
          </CardContent>
        </Card>
      ) : null}
    </>
  );
}

function SecurityEffectQueue({
  diagnostics,
}: {
  readonly diagnostics: SecurityEffectDiagnostics | undefined;
}) {
  useUiTranslation();
  if (!diagnostics) return null;
  const needsAttention =
    (diagnostics.oldest_pending_age_seconds ?? 0) >= 3600 ||
    diagnostics.counts.some(
      (item) => item.state === "uncertain" || item.state === "failed",
    );
  return (
    <Card className={needsAttention ? "border-destructive/50" : undefined}>
      <CardHeader>
        <CardTitle className="text-15">{translateUi("Security effect relay")}</CardTitle>
        {needsAttention ? (
          <Badge variant="destructive">{translateUi("Action required")}</Badge>
        ) : null}
      </CardHeader>
      <CardContent className="space-y-2 text-14">
        <p className="text-muted-foreground">
          {diagnostics.oldest_pending_at
            ? translateUi("Oldest outstanding effect: {{value1}}", {
                value1: `${formatTimestamp(diagnostics.oldest_pending_at)} (${diagnostics.oldest_pending_age_seconds ?? 0}s)`,
              })
            : translateUi("No outstanding effects")}
        </p>
        {diagnostics.counts.map((item) => (
          <div key={`${item.kind}:${item.state}`} className="flex justify-between gap-4">
            <span>{item.kind} / {item.state}</span>
            <span className="font-mono">
              {item.count}{item.at_least ? "+" : ""}
            </span>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

export function AdminIntegrityPage() {
  useUiTranslation();
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
        if (broken.length === 0) {
          toast.success(translateUi("Verification passed: both chains intact"));
        } else {
          toast.error(
            translateErrorMessage("Integrity break detected in {{chains}}", { chains: broken.map((chain) => translateUi(CHAIN_LABELS[chain.chain])).join(", ") }),
          );
        }
      },
      onError: () => toast.error(translateErrorMessage("Verification run failed")),
    });
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title={translateUi("Integrity")}
        description={translateUi("Tamper-evidence status of the audit log and billing ledger hash chains, re-verified automatically in the background.")}
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
              />{translateUi("Run check now")}</Button>
          ) : undefined
        }
      />

      {data ? (
        <StartupDiagnostics diagnostics={data.startup_diagnostics} />
      ) : null}
      {data ? <SecurityEffectQueue diagnostics={data.security_effects} /> : null}

      {error ? (
        <Card>
          <CardContent className="py-8 text-center text-14 text-muted-foreground">{translateUi("Failed to load chain verification status.")}</CardContent>
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
          <CardContent className="py-8 text-center text-14 text-muted-foreground">{translateUi("No verification runs recorded yet. The background sweep runs on an interval after startup; use Run check now to verify immediately.")}</CardContent>
        </Card>
      )}
    </div>
  );
}
