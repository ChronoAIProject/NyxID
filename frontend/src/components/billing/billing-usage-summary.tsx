import { Card } from "@/components/ui/card";
import type { BillingUsageRow } from "@/schemas/billing";
import { number, creditsLabel, creditsSumLabel } from "@/lib/billing-display";
import { metricTotals } from "@/lib/billing-usage";

export function BillingUsageSummary({ rows }: { rows: BillingUsageRow[] }) {
  const metrics = new Map(metricTotals(rows));
  const estimated = creditsLabel(rows, "estimated_credits_micros");
  const covered = creditsSumLabel(rows, [
    "grant_credits_micros",
    "allowance_credits_micros",
  ]);
  const tokenTotal = metrics.get("tokens");
  return (
    <Card className="tab-usage-summary">
      <div className="summary-family">
        <h3>Spend</h3>
        <strong>
          {estimated.text} <small>credits</small>
        </strong>
        <p>
          Estimated usage cost
          {estimated.partial &&
            ` · lower bound, ${estimated.unknown} ${estimated.unknown === 1 ? "record" : "records"} unpriced`}
        </p>
        <dl>
          <div>
            <dt>Covered by benefits</dt>
            <dd>{covered.text}</dd>
          </div>
          <div>
            <dt>Wallet-funded</dt>
            <dd>{creditsLabel(rows, "wallet_credits_micros").text}</dd>
          </div>
        </dl>
      </div>
      <div className="summary-family">
        <h3>Activity</h3>
        <strong>
          {number(metrics.get("requests") ?? 0)} <small>requests</small>
        </strong>
        <p>Metered requests</p>
        <dl>
          <div>
            <dt>Services used</dt>
            <dd>
              {
                new Set(rows.map((row) => row.service_id ?? row.service_slug))
                  .size
              }
            </dd>
          </div>
          <div>
            <dt>Images · Data</dt>
            <dd>
              {number(metrics.get("images") ?? 0)} ·{" "}
              {number(metrics.get("bytes") ?? 0)} bytes
            </dd>
          </div>
        </dl>
      </div>
      <div className="summary-family">
        <h3>Tokens</h3>
        <strong>
          {tokenTotal != null ? number(tokenTotal) : "—"}{" "}
          <small>{tokenTotal != null ? "tokens" : "No total metered"}</small>
        </strong>
        <p>Metered token usage</p>
        <dl>
          <div>
            <dt>Input · Output</dt>
            <dd>
              {metrics.has("input_tokens")
                ? number(metrics.get("input_tokens")!)
                : "—"}{" "}
              ·{" "}
              {metrics.has("output_tokens")
                ? number(metrics.get("output_tokens")!)
                : "—"}
            </dd>
          </div>
          <div>
            <dt>Cache read · write</dt>
            <dd>
              {metrics.has("cache_read_tokens")
                ? number(metrics.get("cache_read_tokens")!)
                : "—"}{" "}
              ·{" "}
              {metrics.has("cache_write_tokens")
                ? number(metrics.get("cache_write_tokens")!)
                : "—"}
            </dd>
          </div>
        </dl>
      </div>
    </Card>
  );
}
