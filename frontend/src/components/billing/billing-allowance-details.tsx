import { useUiTranslation } from "@/i18n/use-ui-translation";
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
  const { t } = useUiTranslation();
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
        aria-label={t("{{label}} used", { label: label })}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={`${formatted}% used${reserved > 0 ? `; ${number(reserved)} reserved` : ""}`}
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
      <span className="benefit-meter-caption">{formatted}{t("% used")}</span>
    </span>
  );
}

const PAGE_SIZE = 5;

function AllowanceGroupDetails({ rows }: { rows: UserAllowanceBalance[] }) {
  const { t } = useUiTranslation();
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
          {rows.length} {rows.length === 1 ? t("allowance") : t("allowances")}
        </span>
        <dl>
          <div>
            <dt>{t("Period starts")}</dt>
            <dd>{timestamp(balance.period_start)}</dd>
          </div>
          <div>
            <dt>{t("Resets / expires")}</dt>
            <dd>
              {balance.period_end
                ? timestamp(balance.period_end)
                : t("No reset or expiry")}
            </dd>
          </div>
        </dl>
        <div
          className="allowance-gauge-legend"
          aria-label={t("Allowance gauge legend")}
        >
          <span>
            <i className="allowance-consumed" />{t("Consumed")}</span>
          <span>
            <i className="allowance-reserved" />{t("Reserved")}</span>
          <span>
            <i className="allowance-available" />{t("Available")}</span>
        </div>
      </div>
      <div className="allowance-balances">
        {rows.slice(first, first + PAGE_SIZE).map((row, index) => (
          <article className="allowance-balance" key={row.allowance.id}>
            <header>
              <strong>{t("Allowance ")}{first + index + 1}</strong>
              <UsedGauge
                used={row.consumed_quantity}
                reserved={row.reserved_quantity}
                limit={row.allowance.quantity}
                label={t("{{value}} allowance {{value1}}", { value: billingMetricLabel(row.allowance.metric), value1: first + index + 1 })}
              />
            </header>
            <dl>
              <div>
                <dt>{t("Limit")}</dt>
                <dd>{number(row.allowance.quantity)}</dd>
              </div>
              <div className="allowance-remaining">
                <dt>{t("Remaining")}</dt>
                <dd>{number(row.remaining_quantity)}</dd>
              </div>
              <div>
                <dt>{t("Consumed")}</dt>
                <dd>{number(row.consumed_quantity)}</dd>
              </div>
              <div>
                <dt>{t("Reserved")}</dt>
                <dd>{number(row.reserved_quantity)}</dd>
              </div>
            </dl>
          </article>
        ))}
      </div>
      {lastPage > 0 && (
        <div className="billing-detail-pagination">
          <span>
            {first + 1}–{Math.min(first + PAGE_SIZE, rows.length)}{t(" of")}{" "}
            {rows.length}{t(" allowances")}</span>
          <Button
            variant="outline"
            size="sm"
            disabled={currentPage === 0}
            onClick={() => setPage(currentPage - 1)}
          >{t("Previous allowances")}</Button>
          <Button
            variant="outline"
            size="sm"
            disabled={currentPage === lastPage}
            onClick={() => setPage(currentPage + 1)}
          >{t("Next allowances")}</Button>
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
  const { t } = useUiTranslation();
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
            (row) => row.consumed_quantity >= row.allowance.quantity,
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
      <div role="region" aria-label={t("Selected allowance balances")}>
        {selected.length === 0 ? (
          <p className="empty-inline">{t("Select metrics to inspect allowance balances.")}</p>
        ) : (
          selected.map(([key, rows]) => (
            <div className="allowance-inspector" key={key}>
              <AllowanceGroupDetails rows={rows} />
            </div>
          ))
        )}
      </div>
      <p className="fine-print mt-3">{t("Each allowance has its own limit. Percentages show consumed usage; reserved units are listed separately.")}</p>
    </div>
  );
}
