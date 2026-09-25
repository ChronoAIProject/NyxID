import { ChevronDown, ChevronRight } from "lucide-react";
import { billingMetricLabel } from "@/lib/billing-units";
import type { BillingUsageRow, BillingUsageTotals } from "@/schemas/billing";
import {
  credits,
  number,
  serviceName,
  serviceCategory,
  total,
  type BillingCatalog,
} from "@/lib/billing-display";
import {
  dimensions,
  groupRows,
  layerName,
  metricTotals,
  metricFamily,
  usageStatus,
  quantitySummary,
  type Dimension,
} from "@/lib/billing-usage";

export function FundingDetails({ rows }: { rows: BillingUsageRow[] }) {
  return (
    <dl className="split-facts funding-facts">
      <div>
        <dt>Estimated cost</dt>
        <dd>{credits(total(rows, "estimated_credits_micros"))} credits</dd>
      </div>
      <div>
        <dt>Funded by credit grants</dt>
        <dd>{credits(total(rows, "grant_credits_micros"))} credits</dd>
      </div>
      <div>
        <dt>Funded by allowances</dt>
        <dd>{credits(total(rows, "allowance_credits_micros"))} credits</dd>
      </div>
      <div>
        <dt>Wallet-funded cost</dt>
        <dd>{credits(total(rows, "wallet_credits_micros"))} credits</dd>
      </div>
      {rows.some((row) =>
        [
          row.estimated_credits_micros,
          row.wallet_credits_micros,
          row.grant_credits_micros,
          row.allowance_credits_micros,
        ].some((value) => value == null),
      ) && (
        <div>
          <dt>Incomplete reporting</dt>
          <dd>
            Some records have no cost or funding data. Expand the records for
            known values.
          </dd>
        </div>
      )}
    </dl>
  );
}
export function MetricDetails({ rows }: { rows: BillingUsageRow[] }) {
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
          <h4>{family}</h4>
          <dl className="split-facts metric-facts">
            {names
              .filter((metric) => metricFamily(metric) === family)
              .map((metric) => (
                <div key={metric}>
                  <dt className="capitalize">{billingMetricLabel(metric)}</dt>
                  <dd>
                    {actual.has(metric)
                      ? number(actual.get(metric)!)
                      : "Not metered"}
                  </dd>
                </div>
              ))}
          </dl>
        </section>
      ))}
      <p className="fine-print">
        Metered quantities, kept in their original units. Total tokens and token
        classes may overlap; they are not added together.
      </p>
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
  return (
    <details className="all-metrics">
      <summary>
        <ChevronDown size={14} className="disclosure-arrow" />
        <strong>All metrics & funding</strong>
        <span>Tokens, caches, images, bytes, and funding sources</span>
      </summary>
      <div className="expanded-metrics">
        <section>
          <h3>Metered quantities</h3>
          <MetricDetails rows={rows} />
        </section>
        <section>
          <h3>Funding breakdown</h3>
          <FundingDetails rows={rows} />
          <AllowanceFunding rows={rows} />
          {totals && (
            <dl className="split-facts">
              <div>
                <dt>Reported requests</dt>
                <dd>{number(totals.requests)}</dd>
              </div>
              <div>
                <dt>Reported bytes</dt>
                <dd>{number(totals.bytes)}</dd>
              </div>
              <div>
                <dt>Meter events</dt>
                <dd>{number(totals.events)}</dd>
              </div>
            </dl>
          )}
          <p className="fine-print">
            Estimated prices and settled funding can differ. Wallet-funded cost
            is separate from rounded wallet debits.
          </p>
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
      <div className="detail-intro">
        <section>
          <h4>Metered quantities</h4>
          <dl className="split-facts">
            {metricTotals(rows).map(([metric, quantity]) => (
              <div key={metric}>
                <dt className="capitalize">{billingMetricLabel(metric)}</dt>
                <dd>{number(quantity)}</dd>
              </div>
            ))}
          </dl>
        </section>
        <section>
          <h4>Funding</h4>
          <FundingDetails rows={rows} />
        </section>
      </div>
      <details className="records-disclosure">
        <summary>
          <ChevronRight size={13} className="disclosure-arrow" />
          <strong>Models, agents & billing layers</strong>
          <span>
            {rows.length} {rows.length === 1 ? "record" : "records"}
          </span>
        </summary>
        <div className="meter-records">
          {rows.map((row, index) => (
            <article className="meter-record" key={index}>
              <header>
                <strong>{serviceName(catalog, row.service_slug)}</strong>
                <span>
                  {row.billable
                    ? row.lago_acked
                      ? "Acknowledged"
                      : "Settlement pending"
                    : "Free"}
                </span>
              </header>
              <dl className="split-facts">
                <div>
                  <dt>Model</dt>
                  <dd>{row.model ?? "No model recorded"}</dd>
                </div>
                <div>
                  <dt>Agent</dt>
                  <dd>
                    {row.api_key_name ??
                      (row.api_key_id
                        ? "Unnamed agent key"
                        : "No agent key recorded")}
                  </dd>
                </div>
                <div>
                  <dt>Billing layer</dt>
                  <dd>{layerName(row.layer)}</dd>
                </div>
                <div>
                  <dt className="capitalize">
                    {billingMetricLabel(row.metric)}
                  </dt>
                  <dd>{number(row.quantity)}</dd>
                </div>
              </dl>
              <details className="record-details">
                <summary>
                  Full metering & funding details{" "}
                  <ChevronDown size={12} className="disclosure-arrow" />
                </summary>
                <FundingDetails rows={[row]} />
                <dl className="split-facts">
                  <div>
                    <dt>Reported requests</dt>
                    <dd>{number(row.requests)}</dd>
                  </div>
                  <div>
                    <dt>Reported bytes</dt>
                    <dd>{number(row.bytes)}</dd>
                  </div>
                  <div>
                    <dt>Meter events</dt>
                    <dd>{number(row.events)}</dd>
                  </div>
                  <div>
                    <dt>Allowance-covered {billingMetricLabel(row.metric)}</dt>
                    <dd>
                      {row.allowance_quantity == null
                        ? "Unavailable"
                        : number(row.allowance_quantity)}
                    </dd>
                  </div>
                  {row.token_breakdown ? (
                    <>
                      <div>
                        <dt>Captured input tokens</dt>
                        <dd>{number(row.token_breakdown.prompt_tokens)}</dd>
                      </div>
                      <div>
                        <dt>Captured output tokens</dt>
                        <dd>{number(row.token_breakdown.completion_tokens)}</dd>
                      </div>
                      <div>
                        <dt>Captured cache-read tokens</dt>
                        <dd>{number(row.token_breakdown.cached_tokens)}</dd>
                      </div>
                      <div>
                        <dt>Captured cache-write tokens</dt>
                        <dd>
                          {number(row.token_breakdown.cache_creation_tokens)}
                        </dd>
                      </div>
                    </>
                  ) : (
                    <div>
                      <dt>Token breakdown</dt>
                      <dd>Not recorded</dd>
                    </div>
                  )}
                  <div>
                    <dt>Meter code</dt>
                    <dd>{row.lago_metric_code}</dd>
                  </div>
                  <div>
                    <dt>Agent key ID</dt>
                    <dd>{row.api_key_id ?? "Not recorded"}</dd>
                  </div>
                </dl>
              </details>
            </article>
          ))}
        </div>
      </details>
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
  const groups = groupRows(catalog, rows, dimension).filter((group) =>
    group.name.toLowerCase().includes(search.trim().toLowerCase()),
  );
  if (!groups.length)
    return (
      <p className="empty-inline">
        No matching usage. Change the filters to see more.
      </p>
    );
  return (
    <div className="expandable-usage">
      <div className="usage-column-labels">
        <span>{dimensions[dimension]}</span>
        <span>Estimated cost</span>
      </div>
      {groups.map((group) => (
        <details className="expandable-service" key={group.key}>
          <summary>
            <ChevronRight size={14} className="disclosure-arrow" />
            <span className="expandable-name">
              <strong>{group.name}</strong>
              <small>{quantitySummary(group.rows)}</small>
              {usageStatus(group.rows).mixed && (
                <small>Includes free usage</small>
              )}
            </span>
            <span className="row-credit">
              {usageStatus(group.rows).label === "Free" ? (
                "—"
              ) : (
                <>
                  {credits(total(group.rows, "estimated_credits_micros"))}{" "}
                  <small>credits</small>
                </>
              )}
              <span
                className={`usage-status ${usageStatus(group.rows).label === "Pending" ? "text-warning" : usageStatus(group.rows).label === "Free" ? "text-success" : ""}`}
              >
                {usageStatus(group.rows).label}
              </span>
            </span>
          </summary>
          <DetailedUsage catalog={catalog} rows={group.rows} />
        </details>
      ))}
    </div>
  );
}

export function AllowanceFunding({ rows }: { rows: BillingUsageRow[] }) {
  const metrics = [...new Set(rows.map((row) => row.metric))];
  return (
    <section className="allowance-funding">
      <h4>Units covered by allowances</h4>
      <dl className="split-facts">
        {metrics.map((metric) => {
          const records = rows.filter((row) => row.metric === metric);
          return (
            <div key={metric}>
              <dt className="capitalize">{billingMetricLabel(metric)}</dt>
              <dd>
                {records.some((row) => row.allowance_quantity == null)
                  ? "Unavailable"
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
  const categories = [
    ...new Set(
      rows.map((row) => serviceCategory(catalog, row.service_slug ?? "")),
    ),
  ].sort();
  if (!rows.length)
    return <p className="empty-inline">No usage in this period.</p>;
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
              {category}
              <span>
                {groupRows(catalog, categoryRows, "service").length}{" "}
                {groupRows(catalog, categoryRows, "service").length === 1
                  ? "service"
                  : "services"}
              </span>
            </h4>
            <ExpandableUsage catalog={catalog} rows={categoryRows} />
          </section>
        );
      })}
    </div>
  );
}
