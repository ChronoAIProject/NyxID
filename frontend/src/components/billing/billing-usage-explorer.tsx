import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { ChevronDown, Search } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { AnalyticsSelect } from "@/components/billing-analytics/filter-picker";
import { type BillingCatalog } from "@/lib/billing-display";
import { dimensions, groupRows, type Dimension } from "@/lib/billing-usage";
import type { BillingUsageRow, BillingUsageTotals } from "@/schemas/billing";
import {
  ExpandableUsage,
  ServiceUsage,
  UsageMetricsDisclosure,
} from "./billing-usage-details";
import { BillingQuantityChart } from "./billing-quantity-chart";
import { BillingFundingChart } from "./billing-funding-chart";

export function BillingUsageExplorer({
  catalog,
  rows,
  totals,
}: {
  catalog: BillingCatalog;
  rows: BillingUsageRow[];
  totals?: BillingUsageTotals;
}) {
  const { t } = useUiTranslation();
  const [dimension, setDimension] = useState<Dimension>("service");
  const [query, setQuery] = useState("");
  const groups = groupRows(catalog, rows, dimension);
  const needle = query.trim().toLowerCase();
  const detailRows = needle
    ? groups
        .filter((group) => group.name.toLowerCase().includes(needle))
        .flatMap((group) => group.rows)
    : rows;
  return (
    <Card className="usage-results usage-explorer">
      <CardHeader className="usage-explorer-heading">
        <div>
          <CardTitle>{t("Usage overview")}</CardTitle>
          <p className="text-12 text-muted-foreground">{t("Compare metered quantities and see how your usage is funded.")}</p>
        </div>
        {rows.length > 0 && (
          <span className="usage-group-count">
            {t("{{count}} groups", { count: groups.length })}
          </span>
        )}
      </CardHeader>
      <CardContent>
        {rows.length === 0 && (
          <p className="empty-inline">{t("No usage in this period.")}</p>
        )}
        {rows.length > 0 && (
          <div className="usage-overview-visuals">
            <BillingQuantityChart rows={rows} catalog={catalog} overview />
            <BillingFundingChart rows={rows} />
          </div>
        )}
        {rows.length > 0 && (
          <UsageMetricsDisclosure rows={rows} totals={totals} />
        )}
        {rows.length > 0 && (
          <details className="usage-group-disclosure">
            <summary>
              <ChevronDown
                size={14}
                className="disclosure-arrow"
                aria-hidden="true"
              />{t("Explore by service, model or agent")}</summary>
            {rows.length > 0 && (
              <div className="usage-explorer-controls">
                <AnalyticsSelect
                  label={t("Group by")}
                  inline
                  value={dimension}
                  onChange={(value) => {
                    setDimension(value as Dimension);
                    setQuery("");
                  }}
                  options={Object.entries(dimensions).map(([value, label]) => ({
                    value,
                    label,
                  }))}
                />
                <div className="usage-search">
                  <Search size={14} aria-hidden="true" />
                  <Input
                    aria-label={t("Search usage groups")}
                    placeholder={t("Search {{dimension}}", { dimension: dimensions[dimension].toLowerCase() })}
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                  />
                </div>
              </div>
            )}
            {!rows.length || (dimension === "service" && !needle) ? (
              <ServiceUsage catalog={catalog} rows={rows} />
            ) : (
              <ExpandableUsage
                catalog={catalog}
                rows={detailRows}
                dimension={dimension}
              />
            )}
          </details>
        )}
      </CardContent>
    </Card>
  );
}
