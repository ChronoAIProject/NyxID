import type {
  CreditGrant,
  UserAllowanceBalance,
} from "@/schemas/billing-credits";
import type { ReactNode } from "react";
import { ChevronDown } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { billingMetricLabel } from "@/lib/billing-units";
import { cn } from "@/lib/utils";
import {
  compact,
  credits,
  number,
  serviceName,
  timestamp,
  type BillingCatalog,
} from "@/lib/billing-display";
import { BenefitHelp, FreeUsageHelp } from "./benefit-help";

/** Allowances shown in a collapsed section; the rest are behind Details. */
const TILE_LIMIT = 6;

function percentUsed(used: number, limit: number) {
  const percent =
    limit > 0 ? Math.min(100, Math.max(0, (used / limit) * 100)) : 0;
  const text =
    percent > 0 && percent < 0.01
      ? "<0.01"
      : number(Math.round(percent * 100) / 100);
  return { percent, text };
}

/**
 * One allowance or grant balance: label, percentage used, a thin bar and a
 * remaining caption. The bar turns warning at 80% and destructive when spent.
 */
function MeterTile({
  label,
  meterLabel,
  used,
  limit,
  caption,
}: {
  label: string;
  meterLabel: string;
  used: number;
  limit: number;
  caption: ReactNode;
}) {
  const { percent, text } = percentUsed(used, limit);
  const level =
    percent >= 100 ? "is-exhausted" : percent >= 80 ? "is-warning" : undefined;
  return (
    <div className={cn("benefit-tile", level)}>
      <span className="benefit-tile-head">
        <span className="benefit-tile-label" title={label}>
          {label}
        </span>
        <span className="benefit-tile-percent">{text}%</span>
      </span>
      <span
        className="benefit-tile-track"
        role="meter"
        aria-label={`${meterLabel} used`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={`${text}% used`}
      >
        <span style={{ width: `${percent}%` }} />
      </span>
      <span className="benefit-tile-caption">{caption}</span>
    </div>
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
      <CardContent>
        <section className="benefit-section">
          {[...grantGroups].map(([key, grants]) => {
            const original = grants.reduce(
              (sum, grant) => sum + grant.amount_micros,
              0,
            );
            const remaining = grants.reduce(
              (sum, grant) => sum + grant.remaining_micros,
              0,
            );
            const reserved = grants.reduce(
              (sum, grant) => sum + grant.reserved_micros,
              0,
            );
            const available = Math.max(0, remaining - reserved);
            const label =
              key === "all"
                ? "All services"
                : grants[0]!.scope.service_slugs
                    .map((slug) => serviceName(catalog, slug))
                    .join(", ");
            return (
              <details className="benefit-disclosure" key={key}>
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
                  <div className="benefit-tiles">
                    <MeterTile
                      label="Credits"
                      meterLabel={`${label} grants`}
                      used={original - remaining}
                      limit={original}
                      caption={
                        <Tooltip delayDuration={150}>
                          <TooltipTrigger asChild>
                            <span className="benefit-tile-balance" tabIndex={0}>
                              {new Intl.NumberFormat("en-US", {
                                maximumFractionDigits: 2,
                              }).format(available / 1_000_000)}{" "}
                              of{" "}
                              {new Intl.NumberFormat("en-US", {
                                maximumFractionDigits: 2,
                              }).format(original / 1_000_000)}{" "}
                              credits left
                            </span>
                          </TooltipTrigger>
                          <TooltipContent>
                            Available: {credits(available)} credits
                          </TooltipContent>
                        </Tooltip>
                      }
                    />
                  </div>
                </summary>
                <div className="benefit-expanded">
                  {grants.map((grant) => (
                    <section key={grant.id}>
                      <h4>{grant.reason || "Credit grant"}</h4>
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
            const tiles = [...units]
              .sort(
                ([, a], [, b]) =>
                  metricRank(a[0]!.allowance.metric) -
                  metricRank(b[0]!.allowance.metric),
              )
              .map(([key, rows]) => {
                const { metric, recurrence } = rows[0]!.allowance;
                const sameMetric = [...units.values()].filter(
                  (other) => other[0]!.allowance.metric === metric,
                ).length;
                const name = sentence(billingMetricLabel(metric));
                return {
                  key,
                  label:
                    sameMetric > 1
                      ? `${name} · ${recurrence.replaceAll("_", " ")}`
                      : name,
                  limit: rows.reduce(
                    (sum, row) => sum + row.allowance.quantity,
                    0,
                  ),
                  consumed: rows.reduce(
                    (sum, row) => sum + row.consumed_quantity,
                    0,
                  ),
                  remaining: rows.reduce(
                    (sum, row) => sum + row.remaining_quantity,
                    0,
                  ),
                };
              });
            const hidden = tiles.length - TILE_LIMIT;
            return (
              <details className="benefit-disclosure" key={id}>
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
                  <div className="benefit-tiles">
                    {tiles.slice(0, TILE_LIMIT).map((tile) => (
                      <MeterTile
                        key={tile.key}
                        label={tile.label}
                        meterLabel={`${serviceName(catalog, slug)} ${tile.label}`}
                        used={tile.consumed}
                        limit={tile.limit}
                        caption={
                          tile.consumed > 0
                            ? `${compact(tile.remaining)} of ${compact(tile.limit)} left`
                            : `Unused · ${compact(tile.remaining)} left`
                        }
                      />
                    ))}
                    {hidden > 0 && (
                      <span className="benefit-tiles-more">+{hidden} more</span>
                    )}
                  </div>
                </summary>
                <div className="benefit-expanded">
                  <p className="fine-print mb-5">
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
