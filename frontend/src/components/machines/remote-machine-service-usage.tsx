import { useState } from "react";
import { useRemoteMachineServiceUsage } from "@/hooks/use-remote-machine-service-usage";
import { creditsLabel, number } from "@/lib/billing-display";
import { exactCredits } from "@/lib/credits";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ErrorBanner } from "@/components/shared/error-banner";

export function RemoteMachineServiceUsage({ machineId }: { readonly machineId: string }) {
  const owner = useAuthStore((state) => state.user?.id);
  const context = `${owner ?? ""}:${machineId}`;
  const [openedFor, setOpenedFor] = useState<string>();
  const opened = openedFor === context;
  const query = useRemoteMachineServiceUsage(machineId, opened);
  const rows = opened && !query.error ? query.data : undefined;
  return <section className="space-y-3" aria-label="Gateway service usage">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <h3 className="text-13 font-semibold">Gateway service usage</h3>
      <Button variant="outline" disabled={!owner || !machineId || (opened && query.isFetching)} onClick={() => { if (opened) void query.refetch(); else setOpenedFor(context); }}>
        {opened ? "Refresh service usage" : "Load service usage"}
      </Button>
    </div>
    <p className="text-12 text-muted-foreground">Last 30 days. Only returned service rows attributed to this Machine are shown. These are existing account service charges, separate from runtime billing; a free Machine does not make gateway services free.</p>
    <p className="text-11 text-muted-foreground">Older or unattributed usage cannot be assigned to this Machine. An empty result does not establish zero service cost. Displayed costs use the existing Billing estimates and are not added to runtime totals.</p>
    {opened && query.isPending && <p role="status" className="text-12">Loading attributed service usage…</p>}
    {opened && query.error && <ErrorBanner message="Service usage could not be verified for this account and period. Retry before relying on this view." onRetry={() => void query.refetch()} />}
    {rows?.length === 0 && <p className="rounded-lg border border-dashed border-border p-3 text-12 text-muted-foreground">No attributed service usage was returned for this Machine in the last 30 days.</p>}
    {rows?.map((row, index) => {
      const attribution = row.machine_usage!;
      const cost = creditsLabel([row], "estimated_credits_micros");
      const exactCost = exactCredits(row.estimated_credits, row.estimated_credits_micros);
      return <article key={`${attribution.job_id}-${row.lago_metric_code}-${row.layer}-${index}`} className="space-y-3 rounded-lg border border-border/50 bg-card p-4">
        <div className="flex flex-wrap items-center gap-2"><h4 className="break-all text-13 font-medium">{row.service_slug ?? row.service_id ?? "Unavailable service"}</h4><Badge variant="secondary">{row.layer}</Badge>{!row.billable && <Badge variant="secondary">Metering only</Badge>}</div>
        <dl className="grid gap-3 text-12 sm:grid-cols-2">
          <div><dt className="text-muted-foreground">Estimated service cost</dt><dd title={row.billable && exactCost !== null ? `${exactCost} Credits` : undefined}>{row.billable ? `${cost.text} Credits` : "Not charged"}</dd></div>
          <div><dt className="text-muted-foreground">Metered quantity</dt><dd>{number(row.quantity)} {row.metric} · {number(row.events)} events</dd></div>
          <div><dt className="text-muted-foreground">Job</dt><dd className="break-all font-mono text-11">{attribution.job_id}</dd></div>
          <div><dt className="text-muted-foreground">Runtime</dt><dd className="break-all font-mono text-11">{attribution.runtime_id}</dd></div>
          <div><dt className="text-muted-foreground">Runtime generation / principal</dt><dd>{attribution.runtime_generation ?? "Unknown"} · {attribution.principal_kind}</dd></div>
          <div><dt className="text-muted-foreground">Grant</dt><dd className="break-all font-mono text-11">{attribution.grant_id ?? "Not reported"}</dd></div>
          {row.operation && <div><dt className="text-muted-foreground">Operation</dt><dd className="break-all">{row.operation}</dd></div>}
          {row.model && <div><dt className="text-muted-foreground">Model</dt><dd className="break-all">{row.model}</dd></div>}
        </dl>
      </article>;
    })}
  </section>;
}
