import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { useRemoteMachineServiceUsage } from "@/hooks/use-remote-machine-service-usage";
import { creditsLabel, number } from "@/lib/billing-display";
import { exactCredits, parseCredits } from "@/lib/credits";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ErrorBanner } from "@/components/shared/error-banner";

export function RemoteMachineServiceUsage({ machineId }: { readonly machineId: string }) {
  useUiTranslation();
  const owner = useAuthStore((state) => state.user?.id);
  const context = `${owner ?? ""}:${machineId}`;
  const [openedFor, setOpenedFor] = useState<string>();
  const opened = openedFor === context;
  const query = useRemoteMachineServiceUsage(machineId, opened);
  const rows = opened && !query.error ? query.data : undefined;
  return <section className="space-y-3" aria-label={translateUi("Gateway service usage")}>
    <div className="flex flex-wrap items-center justify-between gap-3">
      <h3 className="type-card-title">{translateUi("Gateway service usage")}</h3>
      <Button variant="outline" disabled={!owner || !machineId || (opened && query.isFetching)} onClick={() => { if (opened) void query.refetch(); else setOpenedFor(context); }}>
        {opened ? translateUi("Refresh service usage") : translateUi("Load service usage")}
      </Button>
    </div>
    <p className="text-14 text-muted-foreground">{translateUi("Last 30 days. Only returned service rows attributed to this Machine are shown. These are existing account service charges, separate from runtime billing; a free Machine does not make gateway services free.")}</p>
    <p className="text-13 text-muted-foreground">{translateUi("Older or unattributed usage cannot be assigned to this Machine. An empty result does not establish zero service cost. Displayed costs use the existing Billing estimates and are not added to runtime totals.")}</p>
    {opened && query.isPending && <p role="status" className="text-14">{translateUi("Loading attributed service usage…")}</p>}
    {opened && query.error && <ErrorBanner message={translateUi("Service usage could not be verified for this account and period. Retry before relying on this view.")} onRetry={() => void query.refetch()} />}
    {rows?.length === 0 && <p className="rounded-lg border border-dashed border-border p-3 text-14 text-muted-foreground">{translateUi("No attributed service usage was returned for this Machine in the last 30 days.")}</p>}
    {rows?.map((row, index) => {
      const attribution = row.machine_usage!;
      const cost = creditsLabel([row], "estimated_credits_micros");
      const exactCost = exactCredits(row.estimated_credits, row.estimated_credits_micros);
      return <article key={`${attribution.job_id}-${row.lago_metric_code}-${row.layer}-${index}`} className="space-y-3 rounded-lg border border-border/50 bg-card p-4">
        <div className="flex flex-wrap items-center gap-2">
          <h4 className="break-all type-card-title">{row.service_slug ?? row.service_id ?? translateUi("Unavailable service")}</h4>
          <Badge variant="secondary">{row.layer}</Badge>
          <Badge variant="secondary">{row.billable ? translateUi("Billable") : translateUi("Metering only")}</Badge>
          {row.billable && (exactCost === null
            ? <Badge variant="warning">{translateUi("Amount unknown")}</Badge>
            : parseCredits(exactCost) === 0n
              ? <Badge variant="secondary">{translateUi("Zero estimate")}</Badge>
              : null)}
        </div>
        <dl className="grid gap-3 text-14 sm:grid-cols-2">
          <div><dt className="text-muted-foreground">{translateUi("Estimated service cost")}</dt><dd title={row.billable && exactCost !== null ? translateUi("{{value1}} Credits", { value1: exactCost }) : undefined}>{row.billable ? translateUi("{{value1}} Credits", { value1: cost.text }) : translateUi("Not charged")}</dd></div>
          <div><dt className="text-muted-foreground">{translateUi("Metered quantity")}</dt><dd>{number(row.quantity)} {row.metric} · {number(row.events)}{translateUi(" events")}</dd></div>
          <div><dt className="text-muted-foreground">{translateUi("Job")}</dt><dd className="break-all font-mono text-13">{attribution.job_id}</dd></div>
          <div><dt className="text-muted-foreground">{translateUi("Runtime")}</dt><dd className="break-all font-mono text-13">{attribution.runtime_id}</dd></div>
          <div><dt className="text-muted-foreground">{translateUi("Runtime generation / principal")}</dt><dd>{attribution.runtime_generation ?? translateUi("Unknown")} · {attribution.principal_kind}</dd></div>
          <div><dt className="text-muted-foreground">{translateUi("Grant")}</dt><dd className="break-all font-mono text-13">{attribution.grant_id ?? translateUi("Not reported")}</dd></div>
          {row.operation && <div><dt className="text-muted-foreground">{translateUi("Operation")}</dt><dd className="break-all">{row.operation}</dd></div>}
          {row.model && <div><dt className="text-muted-foreground">{translateUi("Model")}</dt><dd className="break-all">{row.model}</dd></div>}
        </dl>
      </article>;
    })}
  </section>;
}
