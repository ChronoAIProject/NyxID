import type {
  CreditGrant,
  UserAllowanceBalance,
} from "@/schemas/billing-credits";
import { ChevronDown } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { billingMetricLabel } from "@/lib/billing-units";
import {
  compact,
  credits,
  number,
  serviceName,
  timestamp,
  type BillingCatalog,
} from "@/lib/billing-display";
import { metricFamily } from "@/lib/billing-usage";
import { StackedMeter, StackSwatch } from "./stacked-meter";
import {
  equalShareStack,
  proportionalStack,
  stackStatusClass,
} from "@/lib/benefit-stack";
import { cn } from "@/lib/utils";
import { BenefitHelp, FreeUsageHelp } from "./benefit-help";

function UsedGauge({
  used,
  limit,
  label,
}: {
  used: number;
  limit: number;
  label: string;
}) {
  const percent =
    limit > 0 ? Math.min(100, Math.max(0, (used / limit) * 100)) : 0;
  const formatted =
    percent > 0 && percent < 0.01
      ? "<0.01"
      : number(Math.round(percent * 100) / 100);
  return (
    <span
      className={cn(
        "benefit-meter",
        percent >= 100 ? "is-exhausted" : percent >= 80 ? "is-warning" : null,
      )}
    >
      <span
        className="benefit-track"
        role="meter"
        aria-label={`${label} used`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={`${formatted}% used`}
      >
        <span style={{ width: `${percent}%` }} />
      </span>
      <span className="benefit-meter-caption">{formatted}% used</span>
    </span>
  );
}

export function BillingBenefitsCard({
  grants: activeGrants,
  allowances,
  catalog,
}: {
  grants: readonly CreditGrant[];
  allowances: readonly UserAllowanceBalance[];
  catalog: BillingCatalog;
}) {
  const grantGroups = new Map<string, CreditGrant[]>();
  for (const grant of activeGrants) {
    const key = grant.scope.all_services
      ? "all"
      : [...grant.scope.service_slugs].sort().join("|");
    grantGroups.set(key, [...(grantGroups.get(key) ?? []), grant]);
  }
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
          {[...grantGroups].map(([key, grants]) => {
            const remaining = grants.reduce(
              (sum, grant) => sum + grant.remaining_micros,
              0,
            );
            const reserved = grants.reduce(
              (sum, grant) => sum + grant.reserved_micros,
              0,
            );
            const label =
              key === "all"
                ? "All services"
                : grants[0]!.scope.service_slugs
                    .map((slug) => serviceName(catalog, slug))
                    .join(", ");
            const stack = proportionalStack(
              grants.map((grant) => ({
                key: grant.id,
                label: grant.reason || "Credit grant",
                short: grant.reason || "Credit grant",
                used: grant.amount_micros - grant.remaining_micros,
                limit: grant.amount_micros,
                legend: `${credits(grant.remaining_micros)} left`,
                detail: `${credits(grant.remaining_micros)} of ${credits(grant.amount_micros)} credits left`,
              })),
            );
            const grantSteps = new Map(
              stack.items.map((item) => [item.key, item.step]),
            );
            return (
              <details
                className={cn(
                  "benefit-disclosure",
                  stackStatusClass(stack.status),
                )}
                key={key}
              >
                <summary className="benefit-summary">
                  <div className="compact-benefit-label">
                    <strong className="benefit-label-with-help">
                      Credit grants
                      <BenefitHelp label="Credit grants">
                        <p>
                          Credits applied after free usage and before your
                          wallet. Grants expiring soonest are used first.
                        </p>
                        <p>
                          {key === "all"
                            ? "These grants cover all services."
                            : `These grants cover ${label}.`}{" "}
                          Reserved credits are excluded from the available
                          balance.
                        </p>
                      </BenefitHelp>
                    </strong>
                    <span>
                      {label} · {grants.length}{" "}
                      {grants.length === 1 ? "grant" : "grants"}
                    </span>
                  </div>
                  <DetailsAction />
                  <div className="benefit-summary-metric">
                    <div className="compact-coverage compact-grant-balance">
                      <Tooltip delayDuration={150}>
                        <TooltipTrigger asChild>
                          <strong tabIndex={0}>
                            {new Intl.NumberFormat("en-US", {
                              maximumFractionDigits: 2,
                            }).format(
                              Math.max(0, remaining - reserved) / 1_000_000,
                            )}{" "}
                            credits
                          </strong>
                        </TooltipTrigger>
                        <TooltipContent>
                          Available:{" "}
                          {credits(Math.max(0, remaining - reserved))} credits
                        </TooltipContent>
                      </Tooltip>
                      <small>available</small>
                    </div>
                    <StackedMeter stack={stack} label={`${label} grants`} />
                  </div>
                </summary>
                <div className="benefit-expanded">
                  {grants.map((grant) => (
                    <section key={grant.id}>
                      <h4 className="stack-labelled">
                        <StackSwatch step={grantSteps.get(grant.id) ?? 0} />
                        {grant.reason || "Credit grant"}
                      </h4>
                      <dl className="split-facts">
                        <div>
                          <dt>Original grant</dt>
                          <dd>{credits(grant.amount_micros)} credits</dd>
                        </div>
                        <div>
                          <dt>Used</dt>
                          <dd>
                            {credits(
                              Math.max(
                                0,
                                grant.amount_micros - grant.remaining_micros,
                              ),
                            )}{" "}
                            credits
                          </dd>
                        </div>
                        <div>
                          <dt>Remaining</dt>
                          <dd>{credits(grant.remaining_micros)} credits</dd>
                        </div>
                        <div>
                          <dt>Reserved</dt>
                          <dd>{credits(grant.reserved_micros)} credits</dd>
                        </div>
                        <div>
                          <dt>Expires</dt>
                          <dd>
                            {grant.expires_at
                              ? timestamp(grant.expires_at)
                              : "No expiry"}
                          </dd>
                        </div>
                        <div>
                          <dt>Issued</dt>
                          <dd>{timestamp(grant.created_at)}</dd>
                        </div>
                        <div>
                          <dt>Status</dt>
                          <dd>
                            {grant.status} ·{" "}
                            {grant.activation_state.replaceAll("_", " ")}
                          </dd>
                        </div>
                      </dl>
                    </section>
                  ))}
                </div>
              </details>
            );
          })}
          {!grantGroups.size && (
            <p className="empty-inline">No active credit grants.</p>
          )}
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
            const unitSteps = new Map(
              stack.items.map((item) => [item.key, item.step]),
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
                  <div className="benefit-allowances">
                    {families.map((family) => (
                      <div className="benefit-unit-family" key={family.name}>
                        <span>{family.name}</span>
                        <div>
                          {family.units.map(([key, rows]) => {
                            const remaining = rows.reduce(
                              (sum, row) => sum + row.remaining_quantity,
                              0,
                            );
                            const limit = rows.reduce(
                              (sum, row) => sum + row.allowance.quantity,
                              0,
                            );
                            const consumed = rows.reduce(
                              (sum, row) => sum + row.consumed_quantity,
                              0,
                            );
                            const metric = rows[0]!.allowance.metric;
                            return (
                              <div className="benefit-unit" key={key}>
                                <span className="capitalize stack-labelled">
                                  <StackSwatch step={unitSteps.get(key) ?? 0} />
                                  {billingMetricLabel(metric)}
                                </span>
                                <span
                                  title={`${number(remaining)} of ${number(limit)} remaining`}
                                >
                                  <strong>{compact(remaining)}</strong> /{" "}
                                  {compact(limit)} left
                                </span>
                                <UsedGauge
                                  used={consumed}
                                  limit={limit}
                                  label={`${serviceName(catalog, slug)} ${billingMetricLabel(metric)}`}
                                />
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    ))}
                  </div>
                  <p className="fine-print mt-3 mb-5">
                    Each allowance has its own limit. Percentages show consumed
                    usage; reservations are listed separately.
                  </p>
                  {balances.map((balance) => (
                    <section key={balance.allowance.id}>
                      <h4 className="capitalize">
                        {billingMetricLabel(balance.allowance.metric)}
                      </h4>
                      <dl className="split-facts">
                        <div>
                          <dt>Limit</dt>
                          <dd>{number(balance.allowance.quantity)}</dd>
                        </div>
                        <div>
                          <dt>Remaining</dt>
                          <dd>{number(balance.remaining_quantity)}</dd>
                        </div>
                        <div>
                          <dt>Consumed</dt>
                          <dd>{number(balance.consumed_quantity)}</dd>
                        </div>
                        <div>
                          <dt>Reserved</dt>
                          <dd>{number(balance.reserved_quantity)}</dd>
                        </div>
                        <div>
                          <dt>Recurrence</dt>
                          <dd>
                            {balance.allowance.recurrence.replaceAll("_", " ")}
                          </dd>
                        </div>
                        <div>
                          <dt>Period starts</dt>
                          <dd>{timestamp(balance.period_start)}</dd>
                        </div>
                        <div>
                          <dt>Resets / expires</dt>
                          <dd>
                            {balance.period_end
                              ? timestamp(balance.period_end)
                              : "No reset or expiry"}
                          </dd>
                        </div>
                      </dl>
                    </section>
                  ))}
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

function DetailsAction() {
  return (
    <span className="compact-benefit-action">
      Details <ChevronDown size={13} className="disclosure-arrow" />
    </span>
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
