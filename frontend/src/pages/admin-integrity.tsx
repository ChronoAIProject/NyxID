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
import type { ChainVerifyStatus } from "@/schemas/admin-chain-verification";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";

const CHAIN_LABELS: Record<ChainVerifyStatus["chain"], string> = {
  audit_log: "Audit Log Chain",
  billing_ledger: "Billing Ledger Chain",
};

const CHAIN_DESCRIPTIONS: Record<ChainVerifyStatus["chain"], string> = {
  audit_log:
    "Hash-chained audit events. A break means a stored audit row was edited, deleted, or reordered after it was written.",
  billing_ledger:
    "Append-only journal of money-moving billing events, with its head anchored into the audit chain so tail truncation is detectable.",
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
            <ShieldCheck className="h-5 w-5 text-emerald-500" />
          )}
          <CardTitle className="text-base">
            {CHAIN_LABELS[status.chain]}
          </CardTitle>
        </div>
        <Badge variant={broken ? "destructive" : "success"}>
          {broken ? "Broken" : "Intact"}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          {CHAIN_DESCRIPTIONS[status.chain]}
        </p>

        {broken ? (
          <div className="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm">
            <p className="font-medium text-destructive">
              Integrity break at seq {status.break_seq ?? "unknown"}
              {status.break_kind ? ` (${status.break_kind})` : ""}
            </p>
            {status.break_detail ? (
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
                {status.break_detail}
              </p>
            ) : null}
            <p className="mt-2 text-muted-foreground">
              Stored history no longer matches its hash chain. Treat as
              possible tampering and investigate before trusting records at
              or after this seq.
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
        if (broken.length === 0) {
          toast.success("Verification passed: both chains intact");
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
          {data.chains.map((status) => (
            <ChainCard key={status.chain} status={status} />
          ))}
        </div>
      ) : (
        <Card>
          <CardContent className="py-8 text-center text-sm text-muted-foreground">
            No verification runs recorded yet. The background sweep runs on
            an interval after startup; use Run check now to verify
            immediately.
          </CardContent>
        </Card>
      )}
    </div>
  );
}
