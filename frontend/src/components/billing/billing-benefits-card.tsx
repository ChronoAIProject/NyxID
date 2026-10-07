import type {
  CreditGrant,
  UserAllowanceBalance,
} from "@/schemas/billing-credits";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { billingMetricLabel } from "@/lib/billing-units";
import {
  compact,
  serviceName,
  type BillingCatalog,
} from "@/lib/billing-display";
import { metricFamily } from "@/lib/billing-usage";
import { StackedMeter, StackSwatch } from "./stacked-meter";
import { CreditGrantsRow } from "./billing-grants-row";
import { DetailsAction } from "./details-action";
import { equalShareStack, stackStatusClass } from "@/lib/benefit-stack";
import { cn } from "@/lib/utils";
import { FreeUsageHelp } from "./benefit-help";
import { BillingAllowanceDetails } from "./billing-allowance-details";

export function BillingBenefitsCard({
  grants: activeGrants,
  allowances,
  catalog,
}: {
  grants: readonly CreditGrant[];
  allowances: readonly UserAllowanceBalance[];
  catalog: BillingCatalog;
}) {
  const services = new Map<string, UserAllowanceBalance[]>();
  for (const balance of allowances) {
    const id = balance.allowance.service_id;
    services.set(id, [...(services.get(id) ?? []), balance]);
  }
  return (
    <Card className="benefits-card compact-benefits">
      <CardHeader>
        <CardTitle>Credit grants & free usage</CardTitle>
      </CardHeader>
      <CardContent className="benefits-container">
        <section className="benefit-section">
          <CreditGrantsRow grants={activeGrants} catalog={catalog} />
        </section>
        <section className="benefit-section">
          {[...services].map(([id, balances]) => {
            const slug = balances[0]!.allowance.service_slug;
            const units = new Map<string, typeof balances>();
            for (const balance of balances) {
              const key = [
                balance.allowance.metric,
                balance.allowance.recurrence,
                balance.period_start,
                balance.period_end,
              ].join("|");
              units.set(key, [...(units.get(key) ?? []), balance]);
            }
            const families = ["Tokens", "Cache", "Requests & other units"]
              .map((family) => ({
                name:
                  family === "Requests & other units" ? "Other usage" : family,
                units: [...units]
                  .sort(
                    ([, a], [, b]) =>
                      metricRank(a[0]!.allowance.metric) -
                      metricRank(b[0]!.allowance.metric),
                  )
                  .filter(
                    ([, rows]) =>
                      metricFamily(rows[0]!.allowance.metric) === family,
                  ),
              }))
              .filter((family) => family.units.length);
            const stack = equalShareStack(
              families
                .flatMap((family) => family.units)
                .map(([key, rows]) => {
                  const metric = rows[0]!.allowance.metric;
                  const limit = rows.reduce(
                    (sum, row) => sum + row.allowance.quantity,
                    0,
                  );
                  const remaining = rows.reduce(
                    (sum, row) => sum + row.remaining_quantity,
                    0,
                  );
                  return {
                    key,
                    label: sentence(billingMetricLabel(metric)),
                    short: shortMetric(metric),
                    used: rows.reduce(
                      (sum, row) => sum + row.consumed_quantity,
                      0,
                    ),
                    limit,
                    legend: `${compact(remaining)} left`,
                    detail: `${compact(remaining)} of ${compact(limit)} left`,
                  };
                }),
            );
            return (
              <details
                className={cn(
                  "benefit-disclosure",
                  stackStatusClass(stack.status),
                )}
                key={id}
              >
                <summary className="benefit-summary">
                  <div className="compact-benefit-label">
                    <strong>{serviceName(catalog, slug)}</strong>
                    <span className="benefit-label-with-help">
                      Free usage
                      <FreeUsageHelp balances={balances} />· {balances.length}{" "}
                      {balances.length === 1 ? "allowance" : "allowances"}
                    </span>
                  </div>
                  <DetailsAction />
                  <div className="benefit-summary-metric">
                    <ul
                      className="compact-coverage stack-legend"
                      aria-label="Remaining free usage"
                    >
                      {stack.entries.map((entry) => (
                        <li
                          key={entry.key}
                          className={cn(
                            entry.status !== "normal" && `is-${entry.status}`,
                          )}
                          title={entry.tooltip}
                        >
                          <StackSwatch step={entry.step} />
                          <span>{entry.short}</span>
                          {entry.legend && <strong>{entry.legend}</strong>}
                          {entry.status !== "normal" && (
                            <span className="sr-only">
                              {entry.status === "exhausted"
                                ? "(used up)"
                                : "(nearly used up)"}
                            </span>
                          )}
                        </li>
                      ))}
                    </ul>
                    <StackedMeter
                      stack={stack}
                      label={`${serviceName(catalog, slug)} free usage`}
                    />
                  </div>
                </summary>
                <div className="benefit-expanded">
                  <BillingAllowanceDetails families={families} />
                </div>
              </details>
            );
          })}
          {!services.size && (
            <p className="empty-inline">No free usage allowances.</p>
          )}
        </section>
      </CardContent>
    </Card>
  );
}

function metricRank(metric: string) {
  const order = [
    "tokens",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "images",
    "requests",
    "bytes",
  ];
  const rank = order.indexOf(metric);
  return rank < 0 ? order.length : rank;
}
function sentence(text: string) {
  return text.charAt(0).toUpperCase() + text.slice(1);
}
function shortMetric(metric: string) {
  return (
    (
      {
        input_tokens: "Input",
        output_tokens: "Output",
        cache_read_tokens: "Cache read",
        cache_write_tokens: "Cache write",
      } as Record<string, string>
    )[metric] ?? sentence(billingMetricLabel(metric))
  );
}
