import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { BillingFundingChart } from "./billing-funding-chart";
import { BillingQuantityChart } from "./billing-quantity-chart";
import { billingMetricLabel } from "@/lib/billing-units";
import type { BillingUsageRow, BillingUsageTotals } from "@/schemas/billing";
import {
  creditsLabel,
  number,
  serviceCategory,
  type BillingCatalog,
} from "@/lib/billing-display";
import {
  dimensions,
  groupRows,
  metricTotals,
  metricFamily,
  usageStatus,
  quantitySummary,
  type Dimension,
} from "@/lib/billing-usage";

export function FundingDetails({ rows }: { rows: BillingUsageRow[] }) {
  const { t } = useUiTranslation();
  const estimated = creditsLabel(rows, "estimated_credits_micros");
  const grants = creditsLabel(rows, "grant_credits_micros");
  const allowances = creditsLabel(rows, "allowance_credits_micros");
  const wallet = creditsLabel(rows, "wallet_credits_micros");
  const unpriced = Math.max(
    estimated.unknown,
    grants.unknown,
    allowances.unknown,
    wallet.unknown,
  );
  return (
    <dl className="split-facts funding-facts">
      <div>
        <dt>{t("Estimated cost")}</dt>
        <dd>{estimated.text}{t(" credits")}</dd>
      </div>
      <div>
        <dt>{t("Funded by credit grants")}</dt>
        <dd>{grants.text}{t(" credits")}</dd>
      </div>
      <div>
        <dt>{t("Funded by allowances")}</dt>
        <dd>{allowances.text}{t(" credits")}</dd>
      </div>
      <div>
        <dt>{t("Wallet-funded cost")}</dt>
        <dd>{wallet.text}{t(" credits")}</dd>
      </div>
      {unpriced > 0 && (
        <div>
          <dt>{t("Incomplete reporting")}</dt>
          <dd>
            {rows.length === 1 ? (
              <>{t("This charged record was metered under a price that is no longer available, so its gross, wallet and allowance costs cannot be estimated. Credit-grant funding is still exact.")}</>
            ) : (
              <>
                {t("{{count}} charged records were metered under a price that is no longer available, so their gross, wallet and allowance costs cannot be estimated. Amounts marked ≥ are lower bounds. Credit-grant funding is still exact.", { count: unpriced })}</>
            )}
          </dd>
        </div>
      )}
    </dl>
  );
}
export function MetricDetails({ rows }: { rows: BillingUsageRow[] }) {
  const { t } = useUiTranslation();
  const actual = new Map(metricTotals(rows));
  const names = [
    ...new Set([
      "requests",
      "tokens",
      "input_tokens",
      "output_tokens",
      "cache_read_tokens",
      "cache_write_tokens",
      "images",
      "bytes",
      ...actual.keys(),
    ]),
  ];
  return (
    <div>
      {["Tokens", "Cache", "Requests & other units"].map((family) => (
        <section className="metric-family" key={family}>
          <h4>{t(family)}</h4>
          <dl className="split-facts metric-facts">
            {names
              .filter((metric) => metricFamily(metric) === family)
              .map((metric) => (
                <div key={metric}>
                  <dt className="capitalize">{billingMetricLabel(metric)}</dt>
                  <dd>
                    {actual.has(metric)
                      ? number(actual.get(metric)!)
                      : t("Not metered")}
                  </dd>
                </div>
              ))}
          </dl>
        </section>
      ))}
      <p className="fine-print">{t("Metered quantities, kept in their original units. Total tokens and token classes may overlap; they are not added together.")}</p>
    </div>
  );
}
export function UsageMetricsDisclosure({
  rows,
  totals,
}: {
  rows: BillingUsageRow[];
  totals?: BillingUsageTotals;
}) {
  const { t } = useUiTranslation();
  return (
    <details className="all-metrics">
      <summary>
        <ChevronDown size={14} className="disclosure-arrow" />
        <strong>{t("All metrics & funding")}</strong>
        <span>{t("Tokens, caches, images, bytes, and funding sources")}</span>
      </summary>
      <div className="expanded-metrics">
        <section>
          <h3>{t("Metered quantities")}</h3>
          <MetricDetails rows={rows} />
        </section>
        <section>
          <h3>{t("Funding breakdown")}</h3>
          <FundingDetails rows={rows} />
          <AllowanceFunding rows={rows} />
          {totals && (
            <dl className="split-facts">
              <div>
                <dt>{t("Reported requests")}</dt>
                <dd>{number(totals.requests)}</dd>
              </div>
              <div>
                <dt>{t("Reported bytes")}</dt>
                <dd>{number(totals.bytes)}</dd>
              </div>
              <div>
                <dt>{t("Meter events")}</dt>
                <dd>{number(totals.events)}</dd>
              </div>
            </dl>
          )}
          <p className="fine-print">{t("Estimated prices and settled funding can differ. Wallet-funded cost is separate from rounded wallet debits.")}</p>
        </section>
      </div>
    </details>
  );
}
export function DetailedUsage({
  catalog,
  rows,
}: {
  catalog: BillingCatalog;
  rows: BillingUsageRow[];
}) {
  return (
    <div className="detailed-usage">
      <div className="usage-visual-grid">
        <BillingQuantityChart rows={rows} catalog={catalog} />
        <BillingFundingChart rows={rows} />
      </div>
      <UsageMetricsDisclosure rows={rows} />
    </div>
  );
}
export function ExpandableUsage({
  catalog,
  rows,
  dimension = "service",
  search = "",
}: {
  catalog: BillingCatalog;
  rows: BillingUsageRow[];
  dimension?: Dimension;
  search?: string;
}) {
  const { t } = useUiTranslation();
  const [openGroups, setOpenGroups] = useState<Set<string>>(() => new Set());
  const groups = groupRows(catalog, rows, dimension).filter((group) =>
    group.name.toLowerCase().includes(search.trim().toLowerCase()),
  );
  if (!groups.length)
    return (
      <p className="empty-inline">{t("No matching usage. Change the filters to see more.")}</p>
    );
  return (
    <div className="expandable-usage">
      <div className="usage-column-labels">
        <span>{dimensions[dimension]}</span>
        <span>{t("Estimated cost")}</span>
      </div>
      {groups.map((group) => (
        <details
          className="expandable-service"
          key={group.key}
          onToggle={(event) => {
            const open = event.currentTarget.open;
            setOpenGroups((current) => {
              if (current.has(group.key) === open) return current;
              const next = new Set(current);
              if (open) next.add(group.key);
              else next.delete(group.key);
              return next;
            });
          }}
        >
          <summary>
            <ChevronRight size={14} className="disclosure-arrow" />
            <span className="expandable-name">
              <strong>{group.name}</strong>
              <small>{quantitySummary(group.rows)}</small>
              {usageStatus(group.rows).mixed && (
                <small>{t("Includes free usage")}</small>
              )}
            </span>
            <span className="row-credit">
              {usageStatus(group.rows).label === "Free" ? (
                "—"
              ) : (
                <>
                  {creditsLabel(group.rows, "estimated_credits_micros").text}{" "}
                  <small>{t("credits")}</small>
                </>
              )}
              <span
                className={`usage-status ${usageStatus(group.rows).label === "Pending" ? "text-warning" : usageStatus(group.rows).label === "Free" ? "text-success" : ""}`}
              >
                {t(usageStatus(group.rows).label)}
              </span>
            </span>
          </summary>
          {openGroups.has(group.key) && (
            <DetailedUsage catalog={catalog} rows={group.rows} />
          )}
        </details>
      ))}
    </div>
  );
}

export function AllowanceFunding({ rows }: { rows: BillingUsageRow[] }) {
  const { t } = useUiTranslation();
  const metrics = [...new Set(rows.map((row) => row.metric))];
  return (
    <section className="allowance-funding">
      <h4>{t("Units covered by allowances")}</h4>
      <dl className="split-facts">
        {metrics.map((metric) => {
          const records = rows.filter((row) => row.metric === metric);
          return (
            <div key={metric}>
              <dt className="capitalize">{billingMetricLabel(metric)}</dt>
              <dd>
                {records.some((row) => row.allowance_quantity == null)
                  ? t("Unavailable")
                  : number(
                      records.reduce(
                        (sum, row) => sum + row.allowance_quantity!,
                        0,
                      ),
                    )}
              </dd>
            </div>
          );
        })}
      </dl>
    </section>
  );
}

export function ServiceUsage({
  catalog,
  rows,
}: {
  catalog: BillingCatalog;
  rows: BillingUsageRow[];
}) {
  const { t } = useUiTranslation();
  const categories = [
    ...new Set(
      rows.map((row) => serviceCategory(catalog, row.service_slug ?? "")),
    ),
  ].sort();
  if (!rows.length)
    return <p className="empty-inline">{t("No usage in this period.")}</p>;
  return (
    <div className="service-categories">
      {categories.map((category) => {
        const categoryRows = rows.filter(
          (row) =>
            serviceCategory(catalog, row.service_slug ?? "") === category,
        );
        return (
          <section className="service-category" key={category}>
            <h4>
              {t(category)}
              <span>
                {groupRows(catalog, categoryRows, "service").length}{" "}
                {groupRows(catalog, categoryRows, "service").length === 1
                  ? t("service")
                  : t("services")}
              </span>
            </h4>
            <ExpandableUsage catalog={catalog} rows={categoryRows} />
          </section>
        );
      })}
    </div>
  );
}
