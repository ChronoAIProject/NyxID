import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Card } from "@/components/ui/card";
import type { BillingUsageRow } from "@/schemas/billing";
import { number, creditsLabel, creditsSumLabel } from "@/lib/billing-display";
import { metricTotals } from "@/lib/billing-usage";

export function BillingUsageSummary({ rows }: { rows: BillingUsageRow[] }) {
  const { t } = useUiTranslation();
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
        <h3>{t("Spend")}</h3>
        <strong>
          {estimated.text} <small>{t("credits")}</small>
        </strong>
        <p>{t("Estimated usage cost")}{estimated.partial &&
            t(" · lower bound, {{unknown}} {{value}} unpriced", { unknown: estimated.unknown, value: estimated.unknown === 1 ? t("record") : t("records") })}
        </p>
        <dl>
          <div>
            <dt>{t("Covered by benefits")}</dt>
            <dd>{covered.text}</dd>
          </div>
          <div>
            <dt>{t("Wallet-funded")}</dt>
            <dd>{creditsLabel(rows, "wallet_credits_micros").text}</dd>
          </div>
        </dl>
      </div>
      <div className="summary-family">
        <h3>{t("Activity")}</h3>
        <strong>
          {number(metrics.get("requests") ?? 0)} <small>{t("requests")}</small>
        </strong>
        <p>{t("Metered requests")}</p>
        <dl>
          <div>
            <dt>{t("Services used")}</dt>
            <dd>
              {
                new Set(rows.map((row) => row.service_id ?? row.service_slug))
                  .size
              }
            </dd>
          </div>
          <div>
            <dt>{t("Images · Data")}</dt>
            <dd>
              {number(metrics.get("images") ?? 0)} ·{" "}
              {number(metrics.get("bytes") ?? 0)}{t(" bytes")}</dd>
          </div>
        </dl>
      </div>
      <div className="summary-family">
        <h3>{t("Tokens")}</h3>
        <strong>
          {tokenTotal != null ? number(tokenTotal) : "—"}{" "}
          <small>{tokenTotal != null ? t("tokens") : t("No total metered")}</small>
        </strong>
        <p>{t("Metered token usage")}</p>
        <dl>
          <div>
            <dt>{t("Input · Output")}</dt>
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
            <dt>{t("Cache read · write")}</dt>
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
