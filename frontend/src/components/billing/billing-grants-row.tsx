import { useState } from "react";
import type { CreditGrant } from "@/schemas/billing-credits";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  formatPercent,
  proportionalStack,
  stackStatus,
  stackStatusClass,
  type StackStatus,
} from "@/lib/benefit-stack";
import {
  credits,
  expiryLabel,
  serviceName,
  type BillingCatalog,
} from "@/lib/billing-display";
import { cn } from "@/lib/utils";
import { BenefitHelp } from "./benefit-help";
import { DetailsAction } from "./details-action";
import { StackedMeter, StackSwatch } from "./stacked-meter";

const toneClass: Record<StackStatus, string | undefined> = {
  normal: undefined,
  warning: "text-warning",
  exhausted: "text-destructive",
};

/** Soonest-expiring first (the order grants are consumed); no expiry last. */
function byConsumption(a: CreditGrant, b: CreditGrant) {
  const expiry = (grant: CreditGrant) =>
    grant.expires_at ? Date.parse(grant.expires_at) : Number.POSITIVE_INFINITY;
  return (
    expiry(a) - expiry(b) || Date.parse(a.created_at) - Date.parse(b.created_at)
  );
}

function grantName(grant: CreditGrant) {
  return grant.reason || "Credit grant";
}

/** Numbers with up to two decimals, as credits are shown elsewhere here. */
const amount = (micros: number) =>
  new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(
    micros / 1_000_000,
  );

function AppliesTo({
  grant,
  catalog,
}: {
  grant: CreditGrant;
  catalog: BillingCatalog;
}) {
  if (grant.scope.all_services) return <>All services</>;
  const names = grant.scope.service_slugs.map((slug) =>
    serviceName(catalog, slug),
  );
  if (names.length <= 2) return <>{names.join(", ")}</>;
  return (
    <Tooltip delayDuration={150}>
      <TooltipTrigger asChild>
        <span className="grant-scope" tabIndex={0}>
          {names.slice(0, 2).join(", ")} +{names.length - 2} more
        </span>
      </TooltipTrigger>
      <TooltipContent>{names.join(", ")}</TooltipContent>
    </Tooltip>
  );
}

function StateBadge({ grant }: { grant: CreditGrant }) {
  if (grant.activation_state === "pending_activation") {
    return <Badge variant="secondary">Pending</Badge>;
  }
  if (grant.status !== "active") {
    return (
      <Badge variant="secondary">
        {grant.status.charAt(0).toUpperCase() + grant.status.slice(1)}
      </Badge>
    );
  }
  return null;
}

/**
 * Every active credit grant, whatever its scope, as one row: a stacked bar
 * with one segment per tranche in consumption order, and a compact table of
 * the tranches under Details.
 */
export function CreditGrantsRow({
  grants: activeGrants,
  catalog,
}: {
  grants: readonly CreditGrant[];
  catalog: BillingCatalog;
}) {
  // Relative expiry hints are relative to when the page was opened.
  const [now] = useState(() => Date.now());
  if (!activeGrants.length) {
    return <p className="empty-inline">No active credit grants.</p>;
  }
  const grants = [...activeGrants].sort(byConsumption);
  // A pending grant is unspendable until its ledger entry is confirmed, so it
  // is listed in the table but kept out of the available balance and the bar.
  const spendable = grants.filter(
    (grant) => grant.activation_state !== "pending_activation",
  );
  const pending = grants
    .filter((grant) => grant.activation_state === "pending_activation")
    .reduce((sum, grant) => sum + grant.remaining_micros, 0);
  const remaining = spendable.reduce(
    (sum, grant) => sum + grant.remaining_micros,
    0,
  );
  const reserved = spendable.reduce(
    (sum, grant) => sum + grant.reserved_micros,
    0,
  );
  const available = Math.max(0, remaining - reserved);
  const stack = proportionalStack(
    spendable.map((grant) => ({
      key: grant.id,
      label: grantName(grant),
      short: grantName(grant),
      used: grant.amount_micros - grant.remaining_micros,
      limit: grant.amount_micros,
      legend: `${credits(grant.remaining_micros)} left`,
      detail: `${credits(grant.remaining_micros)} of ${credits(grant.amount_micros)} credits left`,
    })),
  );
  const rows = grants.map((grant) => {
    const item = stack.items.find((entry) => entry.key === grant.id);
    const percent = item?.percent ?? 0;
    return {
      grant,
      step: item?.step ?? null,
      percent,
      status: stackStatus(percent),
    };
  });
  return (
    <details
      className={cn("benefit-disclosure", stackStatusClass(stack.status))}
    >
      <summary className="benefit-summary">
        <div className="compact-benefit-label">
          <strong className="benefit-label-with-help">
            Credit grants
            <BenefitHelp label="Credit grants">
              <p>
                Credits applied after free usage and before your wallet. Grants
                expiring soonest are used first.
              </p>
              <p>
                Reserved credits are excluded from the available balance.
                Pending grants become available once they are activated.
              </p>
            </BenefitHelp>
          </strong>
          <span>
            {grants.length} {grants.length === 1 ? "grant" : "grants"}
          </span>
        </div>
        <DetailsAction />
        <div className="benefit-summary-metric">
          <div className="compact-coverage compact-grant-balance">
            <Tooltip delayDuration={150}>
              <TooltipTrigger asChild>
                <strong tabIndex={0}>{amount(available)} credits</strong>
              </TooltipTrigger>
              <TooltipContent>
                Available: {credits(available)} credits
              </TooltipContent>
            </Tooltip>
            <small>available</small>
            {pending > 0 && <small>· {amount(pending)} pending</small>}
          </div>
          <StackedMeter stack={stack} label="Credit grants" />
        </div>
      </summary>
      <div className="benefit-expanded">
        <div className="grants-desktop overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Grant</TableHead>
                <TableHead>Applies to</TableHead>
                <TableHead className="text-right">Remaining</TableHead>
                <TableHead className="text-right">Used</TableHead>
                <TableHead>Expires</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map(({ grant, step, percent, status }) => (
                <TableRow key={grant.id}>
                  <TableCell>
                    <span className="stack-labelled">
                      <StackSwatch step={step} />
                      {grantName(grant)}
                      <StateBadge grant={grant} />
                    </span>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    <AppliesTo grant={grant} catalog={catalog} />
                  </TableCell>
                  <TableCell className="text-right tabular-nums">
                    {amount(grant.remaining_micros)} of{" "}
                    {amount(grant.amount_micros)}
                    {grant.reserved_micros > 0 && (
                      <span className="block text-[11px] text-muted-foreground">
                        {amount(grant.reserved_micros)} reserved
                      </span>
                    )}
                  </TableCell>
                  <TableCell
                    className={cn("text-right tabular-nums", toneClass[status])}
                  >
                    {formatPercent(percent)}%
                  </TableCell>
                  <TableCell className="whitespace-nowrap">
                    {expiryLabel(grant.expires_at, now)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
        <ul className="grants-mobile" aria-label="Credit grants">
          {rows.map(({ grant, step, percent, status }) => (
            <li key={grant.id}>
              <span className="stack-labelled">
                <StackSwatch step={step} />
                <strong>{grantName(grant)}</strong>
                <StateBadge grant={grant} />
              </span>
              <span className={cn("tabular-nums", toneClass[status])}>
                {formatPercent(percent)}% used
              </span>
              <small>
                {amount(grant.remaining_micros)} of{" "}
                {amount(grant.amount_micros)} left
                {grant.reserved_micros > 0 &&
                  ` · ${amount(grant.reserved_micros)} reserved`}{" "}
                · <AppliesTo grant={grant} catalog={catalog} /> ·{" "}
                {expiryLabel(grant.expires_at, now)}
              </small>
            </li>
          ))}
        </ul>
      </div>
    </details>
  );
}
