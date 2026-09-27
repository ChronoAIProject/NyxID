import { useDeferredValue, useId, useState } from "react";
import { Check, ChevronDown, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
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

export function AnalyticsSelect({
  label,
  value,
  options,
  onChange,
  inline = false,
}: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  inline?: boolean;
}) {
  const id = useId();
  return (
    <div
      className={cn(
        "min-w-0",
        inline ? "flex items-center gap-2" : "space-y-1.5",
      )}
    >
      <label
        htmlFor={id}
        className="whitespace-nowrap text-[10px] font-medium text-muted-foreground"
      >
        {label}
      </label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger
          id={id}
          aria-label={label}
          className={inline ? "w-40" : "w-full"}
          style={inline ? { marginTop: 0 } : undefined}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}
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
function FilterPicker({
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
  const [draft, setDraft] = useState(values);
  const deferred = useDeferredValue(search);
  const label = FILTER_FIELDS.find((field) => field.key === kind)!.label;
  const query = useAnalyticsOptions(kind, deferred, open, sample);
  const matches = (value: string, option: { id: string; detail?: string }) =>
    value === option.id || (kind === "services" && value === option.detail);
  const selected = (option: { id: string; detail?: string }) =>
    draft.some((value) => matches(value, option));
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (next) {
          setDraft(values);
          setSearch("");
        }
        onOpenChange(next);
      }}
    >
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          aria-label={`Filter ${label.toLowerCase()}`}
          className="justify-between"
        >
          {label}
          <ChevronDown className="size-3" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 space-y-3 p-3">
        <div className="flex items-center justify-between">
          <span className="text-[12px] font-medium">{label}</span>
          <span className="text-[11px] text-muted-foreground">
            {draft.length} selected
          </span>
        </div>
        <div className="relative">
          <Search className="absolute left-2.5 top-2 size-3.5 text-muted-foreground" />
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={
              kind === "actors"
                ? "Search by email"
                : kind === "owners"
                  ? "Organization name, slug, or email"
                  : "Search services"
            }
            aria-label={`Search ${label.toLowerCase()}`}
            className="pl-8"
          />
        </div>
        {query.isPending ? (
          <Skeleton className="h-24" />
        ) : query.isError ? (
          <ErrorBanner
            message={`Could not load ${label.toLowerCase()}.`}
            onRetry={() => void query.refetch()}
          />
        ) : (
          <div className="max-h-64 space-y-1 overflow-auto">
            {query.data.options.map((option) => (
              <Button
                key={option.id}
                variant="ghost"
                className="h-auto w-full justify-start py-2 text-left"
                disabled={!selected(option) && draft.length >= 20}
                aria-pressed={selected(option)}
                onClick={() =>
                  setDraft(
                    selected(option)
                      ? draft.filter((value) => !matches(value, option))
                      : [...draft, option.id],
                  )
                }
              >
                <span className="w-3 shrink-0">
                  {selected(option) && <Check className="size-3" />}
                </span>
                <span className="min-w-0">
                  <span className="block truncate">{option.label}</span>
                  {option.detail && (
                    <span className="block truncate text-[10px] text-muted-foreground">
                      {option.detail}
                    </span>
                  )}
                </span>
              </Button>
            ))}
            {query.data.options.length === 0 && (
              <p className="p-2 text-[12px] text-muted-foreground">
                No matches.
              </p>
            )}
            {query.data.total > query.data.options.length && (
              <p className="text-[10px] text-muted-foreground">
                Search to narrow {query.data.total.toLocaleString()} matches.
              </p>
            )}
          </div>
        )}
        <div className="flex items-center gap-2 border-t border-border/60 pt-3">
          <Button
            variant="ghost"
            size="sm"
            disabled={!draft.length}
            onClick={() => setDraft([])}
          >
            Clear
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="ml-auto"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={() => {
              onChange(draft);
              onOpenChange(false);
            }}
          >
            Apply
          </Button>
        </div>
      </PopoverContent>
    </Popover>
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
    <div className="space-y-3 rounded-xl border border-border/50 bg-card px-4 py-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-2">
          {(["services", "owners", "actors"] as const).map((kind) => (
            <FilterPicker
              kind={kind}
              key={`${kind}:${openKind === kind}:${filters[kind].join(",")}`}
              open={openKind === kind}
              onOpenChange={(open) => setOpenKind(open ? kind : null)}
              values={filters[kind]}
              onChange={(values) => onChange({ ...filters, [kind]: values })}
              sample={sample}
            />
          ))}
        </div>
        <div className="min-w-40">
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
        </div>
      </div>
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
    </div>
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
