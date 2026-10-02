import { useDeferredValue, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AnalyticsSelect, FilterCard, FilterPicker } from "./filter-picker";
export { AnalyticsSelect } from "./filter-picker";
import {
  useAnalyticsOptions,
  useAnalyticsLabels,
  type SampleOptions,
} from "@/hooks/use-usage-analytics";
import { DataTableFilterChips } from "@/components/data-table/data-table-controls";
import type { DataTableFilterField } from "@/types/data-table";
export type { SampleOptions } from "@/hooks/use-usage-analytics";
import {
  ANALYTICS_MEASURES,
  ANALYTICS_INTERVALS,
  CHART_TYPES,
  type AnalyticsFilters,
  type AnalyticsPanel,
} from "@/schemas/usage-analytics";
import { BILLING_METRICS, metricLabel } from "@/schemas/billing-metrics";
import {
  BREAKDOWN_LABELS,
  EMPTY_FILTERS,
  MEASURE_LABELS,
  MEASURE_DESCRIPTIONS,
  INTERVAL_LABELS,
  filterError,
} from "@/lib/usage-analytics";

type FilterKind = keyof SampleOptions;
const FILTER_FIELDS: DataTableFilterField<FilterKind>[] = [
  {
    key: "services",
    label: "Services",
    value_type: "enum",
    operator: "includes",
    multiple: true,
    options: [],
  },
  {
    key: "owners",
    label: "Billing accounts",
    value_type: "enum",
    operator: "includes",
    multiple: true,
    options: [],
  },
  {
    key: "actors",
    label: "Acting users",
    value_type: "enum",
    operator: "includes",
    multiple: true,
    options: [],
  },
];
function AnalyticsFilterPicker({
  kind,
  values,
  onChange,
  sample,
  open,
  onOpenChange,
}: {
  kind: FilterKind;
  values: string[];
  onChange: (values: string[]) => void;
  sample?: SampleOptions;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [search, setSearch] = useState("");
  const deferred = useDeferredValue(search);
  const label = FILTER_FIELDS.find((field) => field.key === kind)!.label;
  const query = useAnalyticsOptions(kind, deferred, true, sample);
  return (
    <FilterPicker
      label={label}
      values={values}
      onChange={onChange}
      open={open}
      onOpenChange={onOpenChange}
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder={
        kind === "actors"
          ? "Search by email"
          : kind === "owners"
            ? "Organization name, slug, or email"
            : "Search services"
      }
      options={
        query.isPending
          ? { status: "pending" }
          : query.isError
            ? { status: "error", onRetry: () => void query.refetch() }
            : {
                status: "success",
                options: query.data.options,
                total: query.data.total,
              }
      }
      matches={(value, option) =>
        value === option.id || (kind === "services" && value === option.detail)
      }
      limit={20}
    />
  );
}
export function FilterBar({
  filters,
  onChange,
  sample,
}: {
  filters: AnalyticsFilters;
  onChange: (filters: AnalyticsFilters) => void;
  sample?: SampleOptions;
}) {
  const error = filterError(filters);
  const [openKind, setOpenKind] = useState<FilterKind | null>(null);
  const labels = useAnalyticsLabels(filters, sample);
  const dateValue = (value: string | null) =>
    value && Number.isFinite(Date.parse(value))
      ? new Date(value).toISOString().slice(0, 16)
      : "";
  const toIso = (value: string) =>
    value && Number.isFinite(Date.parse(`${value}Z`))
      ? new Date(`${value}Z`).toISOString()
      : null;
  return (
    <FilterCard
      pickers={(["services", "owners", "actors"] as const).map((kind) => (
        <AnalyticsFilterPicker
          kind={kind}
          key={`${kind}:${openKind === kind}:${filters[kind].join(",")}`}
          open={openKind === kind}
          onOpenChange={(open) => setOpenKind(open ? kind : null)}
          values={filters[kind]}
          onChange={(values) => onChange({ ...filters, [kind]: values })}
          sample={sample}
        />
      ))}
      aside={
        <AnalyticsSelect
          label="Time range"
          inline
          value={filters.period}
          onChange={(value) =>
            onChange({
              ...filters,
              period: value as AnalyticsFilters["period"],
              from:
                value === "custom"
                  ? new Date(
                      Date.now() -
                        { "24h": 1, "7d": 7, "30d": 30, custom: 7 }[
                          filters.period
                        ] *
                          86_400_000,
                    ).toISOString()
                  : null,
              to: value === "custom" ? new Date().toISOString() : null,
            })
          }
          options={[
            { value: "24h", label: "Last 24 hours" },
            { value: "7d", label: "Last 7 days" },
            { value: "30d", label: "Last 30 days" },
            { value: "custom", label: "Custom range" },
          ]}
        />
      }
    >
      <DataTableFilterChips
        search=""
        searchFields={[]}
        searchFilters={[]}
        filters={FILTER_FIELDS.filter(
          (field) => filters[field.key].length > 0,
        ).map((field) => ({
          field,
          values: filters[field.key],
          valueLabels: filters[field.key].map(
            (value) => labels[field.key][value] ?? value,
          ),
        }))}
        onEditSearch={() => undefined}
        onRemoveSearch={() => undefined}
        onEditSearchValue={() => undefined}
        onRemoveSearchValue={() => undefined}
        onEdit={setOpenKind}
        onRemove={(kind) => onChange({ ...filters, [kind]: [] })}
        onClear={() =>
          onChange({ ...filters, services: [], actors: [], owners: [] })
        }
      />
      {filters.period === "custom" && (
        <div className="flex flex-wrap items-end gap-3">
          {(["from", "to"] as const).map((bound) => (
            <label
              key={bound}
              className="flex items-center gap-2 whitespace-nowrap text-[11px] text-muted-foreground"
            >
              <span>
                {bound === "from" ? "Start (UTC)" : "End (UTC, exclusive)"}
              </span>
              <Input
                className="w-auto"
                type="datetime-local"
                aria-label={bound === "from" ? "Start (UTC)" : "End (UTC)"}
                value={dateValue(filters[bound])}
                onChange={(e) =>
                  onChange({ ...filters, [bound]: toIso(e.target.value) })
                }
              />
            </label>
          ))}
          <Button
            variant="ghost"
            onClick={() =>
              onChange({
                ...filters,
                period: EMPTY_FILTERS.period,
                from: null,
                to: null,
              })
            }
          >
            Reset range
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="text-[12px] text-warning">
          {error}
        </p>
      )}
    </FilterCard>
  );
}
export function PanelControls({
  panel,
  onChange,
}: {
  panel: AnalyticsPanel;
  onChange: (panel: AnalyticsPanel) => void;
}) {
  return (
    <div className="space-y-4">
      <label className="block space-y-1.5 text-[10px] font-medium text-muted-foreground">
        Panel title
        <Input
          value={panel.title}
          maxLength={100}
          onChange={(e) => onChange({ ...panel, title: e.target.value })}
        />
      </label>
      <AnalyticsSelect
        label="Measure"
        value={panel.measure}
        onChange={(measure) =>
          onChange({
            ...panel,
            measure: measure as AnalyticsPanel["measure"],
            chart:
              measure === "requests" && panel.chart === "combo"
                ? "line"
                : panel.chart,
          })
        }
        options={ANALYTICS_MEASURES.map((value) => ({
          value,
          label: MEASURE_LABELS[value],
        }))}
      />
      <p className="text-[11px] leading-relaxed text-muted-foreground">
        {MEASURE_DESCRIPTIONS[panel.measure]}
      </p>
      {panel.measure === "quantity" && (
        <AnalyticsSelect
          label="Metric unit"
          value={panel.metric}
          onChange={(metric) =>
            onChange({ ...panel, metric: metric as AnalyticsPanel["metric"] })
          }
          options={BILLING_METRICS.map((value) => ({
            value,
            label: metricLabel(value),
          }))}
        />
      )}
      <AnalyticsSelect
        label="Chart type"
        value={panel.chart}
        onChange={(chart) =>
          onChange({ ...panel, chart: chart as AnalyticsPanel["chart"] })
        }
        options={CHART_TYPES.filter(
          (value) => value !== "combo" || panel.measure !== "requests",
        ).map((value) => ({
          value,
          label: {
            line: "Line",
            bar: "Bar",
            pie: "Donut / pie",
            combo: "Combined bar + line",
          }[value],
        }))}
      />
      <AnalyticsSelect
        label="Data table"
        value={panel.table_display ?? "always"}
        onChange={(table_display) =>
          onChange({
            ...panel,
            table_display: table_display as AnalyticsPanel["table_display"],
          })
        }
        options={[
          { value: "always", label: "Always open" },
          { value: "accordion", label: "Accordion" },
        ]}
      />
      <AnalyticsSelect
        label="Break down by"
        value={panel.breakdown}
        onChange={(breakdown) =>
          onChange({
            ...panel,
            breakdown: breakdown as AnalyticsPanel["breakdown"],
          })
        }
        options={Object.entries(BREAKDOWN_LABELS).map(([value, label]) => ({
          value,
          label,
        }))}
      />
      <AnalyticsSelect
        label="Show"
        value={String(panel.top)}
        onChange={(top) =>
          onChange({ ...panel, top: Number(top) as AnalyticsPanel["top"] })
        }
        options={[
          { value: "5", label: "Top 5 + Other" },
          { value: "10", label: "Top 10 + Other" },
          { value: "20", label: "Top 20 + Other" },
          { value: "0", label: "Aggregated total" },
        ]}
      />
      {(panel.chart === "line" || panel.chart === "combo") && (
        <>
          <AnalyticsSelect
            label="Time interval"
            value={panel.interval ?? "auto"}
            onChange={(interval) =>
              onChange({
                ...panel,
                interval: interval as AnalyticsPanel["interval"],
              })
            }
            options={ANALYTICS_INTERVALS.map((value) => ({
              value,
              label: INTERVAL_LABELS[value],
            }))}
          />
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            Weeks start Monday; months follow the calendar, in UTC. Edge buckets
            include only the selected dates.
          </p>
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            Trends use the same Top 5/10 groups across the whole window. Other
            keeps the remaining usage visible. Combined charts stack these
            groups with total requests on the right axis.
          </p>
        </>
      )}
    </div>
  );
}
