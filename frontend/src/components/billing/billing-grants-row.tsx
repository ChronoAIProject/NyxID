import { useUiTranslation } from "@/i18n/use-ui-translation";
import {
  decimalCredits,
  exactCredits,
  formatExactCredits,
  parseCredits,
} from "@/lib/credits";
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

const grantPico = (
  grant: CreditGrant,
  field: "amount" | "remaining" | "reserved",
) => parseCredits(exactCredits(grant[field], grant[`${field}_micros`]) ?? "0");
const amount = (pico: bigint) => formatExactCredits(decimalCredits(pico), 2);
const exact = (pico: bigint) => credits(decimalCredits(pico));

function AppliesTo({
  grant,
  catalog,
}: {
  grant: CreditGrant;
  catalog: BillingCatalog;
}) {
  const { t } = useUiTranslation();
  if (grant.scope.all_services) return <>{t("All services")}</>;
  const names = grant.scope.service_slugs.map((slug) =>
    serviceName(catalog, slug),
  );
  if (names.length <= 2) return <>{names.join(", ")}</>;
  return (
    <Tooltip delayDuration={150}>
      <TooltipTrigger asChild>
        <span className="grant-scope" tabIndex={0}>
          {names.slice(0, 2).join(", ")} +{names.length - 2}{t(" more")}</span>
      </TooltipTrigger>
      <TooltipContent>{names.join(", ")}</TooltipContent>
    </Tooltip>
  );
}

function StateBadge({ grant }: { grant: CreditGrant }) {
  const { t } = useUiTranslation();
  if (grant.activation_state === "pending_activation") {
    return <Badge variant="secondary">{t("Pending")}</Badge>;
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
  const { t } = useUiTranslation();
  // Relative expiry hints are relative to when the page was opened.
  const [now] = useState(() => Date.now());
  if (!activeGrants.length) {
    return <p className="empty-inline">{t("No active credit grants.")}</p>;
  }
  const grants = [...activeGrants].sort(byConsumption);
  // A pending grant is unspendable until its ledger entry is confirmed, so it
  // is listed in the table but kept out of the available balance and the bar.
  const spendable = grants.filter(
    (grant) => grant.activation_state !== "pending_activation",
  );
  const pending = grants
    .filter((grant) => grant.activation_state === "pending_activation")
    .reduce((sum, grant) => sum + grantPico(grant, "remaining"), 0n);
  const remaining = spendable.reduce(
    (sum, grant) => sum + grantPico(grant, "remaining"),
    0n,
  );
  const reserved = spendable.reduce(
    (sum, grant) => sum + grantPico(grant, "reserved"),
    0n,
  );
  const available = remaining > reserved ? remaining - reserved : 0n;
  const limit = spendable.reduce(
    (sum, grant) => sum + grantPico(grant, "amount"),
    0n,
  );
  // Only normalized drawing proportions become floating point. Money stays bigint.
  const geometry = (value: bigint) =>
    limit === 0n ? 0 : Number((value * 1_000_000_000n) / limit);
  const stack = proportionalStack(
    spendable.map((grant) => ({
      key: grant.id,
      label: grantName(grant),
      short: grantName(grant),
      used: geometry(
        grantPico(grant, "amount") - grantPico(grant, "remaining"),
      ),
      limit: geometry(grantPico(grant, "amount")),
      legend: t("{{amount}} left", { amount: exact(grantPico(grant, "remaining")) }),
      detail: t("{{remaining}} of {{limit}} credits left", { remaining: exact(grantPico(grant, "remaining")), limit: exact(grantPico(grant, "amount")) }),
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
          <strong className="benefit-label-with-help">{t("Credit grants")}<BenefitHelp label={t("Credit grants")}>
              <p>{t("Credits applied after free usage and before your wallet. Grants expiring soonest are used first.")}</p>
              <p>{t("Reserved credits are excluded from the available balance. Pending grants become available once they are activated.")}</p>
            </BenefitHelp>
          </strong>
          <span>
            {grants.length} {grants.length === 1 ? t("grant") : t("grants")}
          </span>
        </div>
        <DetailsAction />
        <div className="benefit-summary-metric">
          <div className="compact-coverage compact-grant-balance">
            <Tooltip delayDuration={150}>
              <TooltipTrigger asChild>
                <strong tabIndex={0}>{amount(available)}{t(" credits")}</strong>
              </TooltipTrigger>
              <TooltipContent>{t("Available: ")}{exact(available)}{t(" credits")}</TooltipContent>
            </Tooltip>
            <small>{t("available")}</small>
            {pending > 0n && <small>· {amount(pending)}{t(" pending")}</small>}
          </div>
          <StackedMeter stack={stack} label={t("Credit grants")} />
        </div>
      </summary>
      <div className="benefit-expanded">
        <div className="grants-desktop overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("Grant")}</TableHead>
                <TableHead>{t("Applies to")}</TableHead>
                <TableHead className="text-right">{t("Remaining")}</TableHead>
                <TableHead className="text-right">{t("Used")}</TableHead>
                <TableHead>{t("Expires")}</TableHead>
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
                    {amount(grantPico(grant, "remaining"))}{t(" of")}{" "}
                    {amount(grantPico(grant, "amount"))}
                    {grantPico(grant, "reserved") > 0n && (
                      <span className="block text-11 text-muted-foreground">
                        {amount(grantPico(grant, "reserved"))}{t(" reserved")}</span>
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
        <ul className="grants-mobile" aria-label={t("Credit grants")}>
          {rows.map(({ grant, step, percent, status }) => (
            <li key={grant.id}>
              <span className="stack-labelled">
                <StackSwatch step={step} />
                <strong>{grantName(grant)}</strong>
                <StateBadge grant={grant} />
              </span>
              <span className={cn("tabular-nums", toneClass[status])}>
                {formatPercent(percent)}{t("% used")}</span>
              <small>
                {amount(grantPico(grant, "remaining"))}{t(" of")}{" "}
                {amount(grantPico(grant, "amount"))}{t(" left")}{grantPico(grant, "reserved") > 0n &&
                  t(" · {{value}} reserved", { value: amount(grantPico(grant, "reserved")) })}{" "}
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
