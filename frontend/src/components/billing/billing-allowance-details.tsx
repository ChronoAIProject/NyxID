import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { billingMetricLabel } from "@/lib/billing-units";
import { number, timestamp } from "@/lib/billing-display";
import { formatPercent, stackStatus } from "@/lib/benefit-stack";
import { cn } from "@/lib/utils";
import type { UserAllowanceBalance } from "@/schemas/billing-credits";
import { BillingMetricPicker } from "./billing-metric-picker";

type AllowanceGroup = [string, UserAllowanceBalance[]];

function UsedGauge({
  used,
  reserved = 0,
  limit,
  label,
}: {
  used: number;
  reserved?: number;
  limit: number;
  label: string;
}) {
  useUiTranslation();
  const percent =
    limit > 0 ? Math.min(100, Math.max(0, (used / limit) * 100)) : 0;
  const formatted = formatPercent(percent);
  const reservedPercent =
    limit > 0
      ? Math.min(100 - percent, Math.max(0, (reserved / limit) * 100))
      : 0;
  return (
    <span
      className={cn(
        "benefit-meter",
        {
          exhausted: "is-exhausted",
          warning: "is-warning",
          normal: null,
        }[stackStatus(percent)],
      )}
    >
      <span
        className="benefit-track allowance-track"
        role="meter"
        aria-label={translateUi("{{value1}} used", { value1: label })}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={`${formatted}% used${reserved > 0 ? `; ${number(reserved)} held` : ""}`}
      >
        <span
          className="allowance-consumed"
          style={{
            width: `${percent}%`,
          }}
        />
        <span
          className="allowance-reserved"
          style={{ width: `${reservedPercent}%` }}
        />
      </span>
      <span className="benefit-meter-caption">{formatted}{translateUi("% used")}</span>
    </span>
  );
}

const PAGE_SIZE = 5;

function AllowanceGroupDetails({ rows }: { rows: UserAllowanceBalance[] }) {
  useUiTranslation();
  const [page, setPage] = useState(0);
  const balance = rows[0]!;
  const lastPage = Math.max(0, Math.ceil(rows.length / PAGE_SIZE) - 1);
  const currentPage = Math.min(page, lastPage);
  const first = currentPage * PAGE_SIZE;
  return (
    <>
      <div className="allowance-period">
        <strong className="capitalize">
          {billingMetricLabel(balance.allowance.metric)}
        </strong>
        <span className="capitalize">
          {balance.allowance.recurrence.replaceAll("_", " ")}
        </span>
        <span>
          {rows.length} {rows.length === 1 ? translateUi("allowance") : translateUi("allowances")}
        </span>
        <dl>
          <div>
            <dt>{translateUi("Period starts")}</dt>
            <dd>{timestamp(balance.period_start)}</dd>
          </div>
          <div>
            <dt>{translateUi("Resets / expires")}</dt>
            <dd>
              {balance.period_end
                ? timestamp(balance.period_end)
                : translateUi("No reset or expiry")}
            </dd>
          </div>
        </dl>
        <div
          className="allowance-gauge-legend"
          aria-label={translateUi("Allowance gauge legend")}
        >
          <span>
            <i className="allowance-consumed" />{translateUi("Consumed")}</span>
          <span>
            <i className="allowance-reserved" />{translateUi("Held")}</span>
          <span>
            <i className="allowance-available" />{translateUi("Available")}</span>
        </div>
      </div>
      <div className="allowance-balances">
        {rows.slice(first, first + PAGE_SIZE).map((row, index) => (
          <article className="allowance-balance" key={row.allowance.id}>
            <header>
              <strong>{translateUi("Allowance ")}{first + index + 1}</strong>
              <UsedGauge
                used={row.consumed_quantity}
                reserved={row.held_quantity ?? row.reserved_quantity}
                limit={row.total_quantity ?? row.allowance.quantity}
                label={translateUi("{{value1}} allowance {{value2}}", { value1: billingMetricLabel(row.allowance.metric), value2: first + index + 1 })}
              />
            </header>
            <dl>
              <div>
                <dt>{translateUi("Limit")}</dt>
                <dd>{number(row.total_quantity ?? row.allowance.quantity)}</dd>
              </div>
              <div className="allowance-remaining">
                <dt>{translateUi("Remaining")}</dt>
                <dd>{number(row.remaining_quantity)}</dd>
              </div>
              <div>
                <dt>{translateUi("Consumed")}</dt>
                <dd>{number(row.consumed_quantity)}</dd>
              </div>
              <div>
                <dt>{translateUi("Held")}</dt>
                <dd>{number(row.held_quantity ?? row.reserved_quantity)}</dd>
              </div>
            </dl>
          </article>
        ))}
      </div>
      {lastPage > 0 && (
        <div className="billing-detail-pagination">
          <span>
            {first + 1}–{Math.min(first + PAGE_SIZE, rows.length)}{translateUi(" of")}{" "}
            {rows.length}{translateUi(" allowances")}</span>
          <Button
            variant="outline"
            size="sm"
            disabled={currentPage === 0}
            onClick={() => setPage(currentPage - 1)}
          >{translateUi("Previous allowances")}</Button>
          <Button
            variant="outline"
            size="sm"
            disabled={currentPage === lastPage}
            onClick={() => setPage(currentPage + 1)}
          >{translateUi("Next allowances")}</Button>
        </div>
      )}
    </>
  );
}

export function BillingAllowanceDetails({
  families,
}: {
  families: { name: string; units: AllowanceGroup[] }[];
}) {
  useUiTranslation();
  const groups = families.flatMap((family) => family.units);
  const [selectedKeys, setSelectedKeys] = useState<string[]>(() =>
    groups[0] ? [groups[0][0]] : [],
  );
  const selected = groups.filter(([key]) => selectedKeys.includes(key));
  if (!groups.length) return null;
  return (
    <div className="allowance-explorer">
      <BillingMetricPicker
        values={selected.map(([key]) => key)}
        onChange={setSelectedKeys}
        options={groups.map(([key, rows]) => {
          const balance = rows[0]!;
          const exhausted = rows.filter(
            (row) =>
              row.consumed_quantity >=
              (row.total_quantity ?? row.allowance.quantity),
          ).length;
          const recurrence = balance.allowance.recurrence.replaceAll("_", " ");
          const label = billingMetricLabel(balance.allowance.metric);
          const sameMetricWindows = groups.filter(
            ([, other]) =>
              other[0]!.allowance.metric === balance.allowance.metric,
          ).length;
          return {
            id: key,
            label:
              sameMetricWindows > 1
                ? `${label} · ${recurrence} · ${timestamp(balance.period_start)}`
                : label,
            detail: `${recurrence} · ${rows.length} ${rows.length === 1 ? "allowance" : "allowances"}${exhausted > 0 ? ` · ${exhausted} used up` : ""}`,
          };
        })}
      />
      <div role="region" aria-label={translateUi("Selected allowance balances")}>
        {selected.length === 0 ? (
          <p className="empty-inline">{translateUi("Select metrics to inspect allowance balances.")}</p>
        ) : (
          selected.map(([key, rows]) => (
            <div className="allowance-inspector" key={key}>
              <AllowanceGroupDetails rows={rows} />
            </div>
          ))
        )}
      </div>
      <p className="fine-print mt-3">{translateUi("Each allowance has its own limit. Percentages show consumed usage; held units are listed separately.")}</p>
    </div>
  );
}
