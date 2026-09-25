import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { billingMetricLabel } from "@/lib/billing-units";
import { number } from "@/lib/billing-display";
import type { UserAllowanceBalance } from "@/schemas/billing-credits";

type AllowanceBalances = UserAllowanceBalance[];

export function AllowanceChart({
  units,
}: {
  units: [string, AllowanceBalances][];
}) {
  const segments = units.map(([key, rows]) => {
    const limit = rows.reduce((sum, row) => sum + row.allowance.quantity, 0);
    const used = rows.reduce((sum, row) => sum + row.consumed_quantity, 0);
    const reserved = rows.reduce((sum, row) => sum + row.reserved_quantity, 0);
    const usedPercent =
      limit > 0 ? Math.min(100, Math.max(0, (used / limit) * 100)) : 0;
    const reservedPercent =
      limit > 0
        ? Math.min(100 - usedPercent, Math.max(0, (reserved / limit) * 100))
        : 0;
    return {
      key,
      label: billingMetricLabel(rows[0]!.allowance.metric),
      usedPercent,
      reservedPercent,
      remainingPercent: limit > 0 ? 100 - usedPercent - reservedPercent : 0,
      percentage:
        limit <= 0
          ? "No limit"
          : usedPercent > 0 && usedPercent < 0.01
            ? "<0.01%"
            : `${number(Math.round(usedPercent * 100) / 100)}%`,
    };
  });
  const usedCount = segments.filter(
    (segment) => segment.usedPercent > 0,
  ).length;
  return (
    <div className="allowance-chart">
      <Tooltip delayDuration={150}>
        <TooltipTrigger asChild>
          <span
            className="allowance-chart-trigger"
            tabIndex={0}
            role="img"
            aria-label={`Allowance usage: ${segments.map((segment) => `${segment.label} ${segment.percentage}`).join(", ")}`}
          >
            <span className="allowance-chart-bar" aria-hidden="true">
              {segments.map((segment) => (
                <span className="allowance-chart-segment" key={segment.key}>
                  <span className="allowance-chart-stack">
                    <span
                      className="allowance-used"
                      style={{ width: `${segment.usedPercent}%` }}
                    />
                    <span
                      className="allowance-reserved"
                      style={{ width: `${segment.reservedPercent}%` }}
                    />
                    <span
                      className="allowance-remaining"
                      style={{ width: `${segment.remainingPercent}%` }}
                    />
                  </span>
                </span>
              ))}
            </span>
            <span className="allowance-chart-status">
              {usedCount ? `${usedCount} of ${segments.length} used` : "Unused"}
            </span>
          </span>
        </TooltipTrigger>
        <TooltipContent
          className="billing-allowance-chart-tooltip"
          side="top"
          collisionPadding={16}
        >
          <p>
            Percentage used of each allowance’s limit, excluding reservations.
          </p>
          <dl>
            {segments.map((segment) => (
              <div key={segment.key}>
                <dt className="capitalize">{segment.label}</dt>
                <dd>{segment.percentage}</dd>
              </div>
            ))}
          </dl>
        </TooltipContent>
      </Tooltip>
    </div>
  );
}
