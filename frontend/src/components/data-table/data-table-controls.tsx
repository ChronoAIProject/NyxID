import {
  Fragment,
  useRef,
  useState,
  type FocusEvent,
  type FormEvent,
  type ReactNode,
  type RefObject,
} from "react";
import {
  Calendar,
  CalendarRange,
  Check,
  ChevronRight,
  Filter,
  Minus,
  Search,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { DatePicker } from "@/components/ui/date-picker";
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
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type {
  AppliedDataTableFilter,
  DataTableFilterField,
  DataTableFilterSelections,
  DataTableSearchApplyMode,
  DataTableSearchField,
  DataTableSearchGroup,
} from "@/types/data-table";

const ALL_FIELDS_VALUE = "__data_table_all_fields__";
const DEFAULT_DATE_MODES = ["dates", "range"] as const;

const FILTER_DATE_FORMATTER = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
  timeZone: "UTC",
});

export interface DataTableSearchProps<SearchKey extends string> {
  readonly fields: readonly DataTableSearchField<SearchKey>[];
  readonly value: string;
  readonly selectedField: SearchKey | null;
  readonly inputRef: RefObject<HTMLInputElement | null>;
  readonly ariaLabel: string;
  readonly allFieldsLabel?: string;
  readonly fieldAriaLabel?: string;
  readonly maxLength?: number;
  readonly onValueChange: (value: string) => void;
  readonly onFieldChange: (field: SearchKey | null) => void;
  readonly onApply: (mode: DataTableSearchApplyMode) => void;
  readonly onCancel: () => void;
}

export function DataTableSearch<SearchKey extends string>({
  fields,
  value,
  selectedField,
  inputRef,
  ariaLabel,
  allFieldsLabel = "All fields",
  fieldAriaLabel = "Search field",
  maxLength = 256,
  onValueChange,
  onFieldChange,
  onApply,
  onCancel,
}: DataTableSearchProps<SearchKey>) {
  const controlRef = useRef<HTMLFormElement>(null);
  const [fieldSelectOpen, setFieldSelectOpen] = useState(false);
  const selectedLabel =
    selectedField === null
      ? allFieldsLabel
      : (fields.find((field) => field.key === selectedField)?.label ??
        selectedField);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    onApply("submit");
  }

  function handleControlBlur(event: FocusEvent<HTMLFormElement>) {
    if (
      event.relatedTarget instanceof Node &&
      controlRef.current?.contains(event.relatedTarget)
    ) {
      return;
    }
    if (fieldSelectOpen) return;
    if (value.trim()) onApply("blur");
  }

  function handleFieldChange(rawValue: string) {
    if (rawValue === ALL_FIELDS_VALUE) {
      onFieldChange(null);
      return;
    }
    const field = fields.find((candidate) => candidate.key === rawValue);
    if (field) onFieldChange(field.key);
  }

  return (
    <form
      ref={controlRef}
      onSubmit={handleSubmit}
      onBlur={handleControlBlur}
      role="search"
      className="w-full min-w-0 flex-1 sm:min-w-[320px]"
    >
      <div className="flex h-11 min-w-0 items-stretch overflow-hidden rounded-lg border border-input bg-transparent transition-colors focus-within:border-white/[0.15] md:h-9">
        <span className="flex w-9 shrink-0 items-center justify-center text-muted-foreground">
          <Search className="h-3.5 w-3.5" aria-hidden="true" />
        </span>
        <input
          ref={inputRef}
          aria-label={ariaLabel}
          placeholder={`Search ${selectedLabel.toLocaleLowerCase()}`}
          value={value}
          maxLength={maxLength}
          enterKeyHint="search"
          onChange={(event) => onValueChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              onCancel();
            }
          }}
          className="h-full min-w-0 flex-1 bg-transparent px-1 text-xs text-foreground outline-none placeholder:text-text-tertiary"
        />
        <button
          type="button"
          aria-label="Clear search draft"
          title="Clear search draft"
          disabled={!value}
          tabIndex={value ? 0 : -1}
          className="flex w-11 shrink-0 items-center justify-center border-l border-border/60 text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:invisible md:w-9"
          onClick={onCancel}
        >
          <X className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
        <Select
          open={fieldSelectOpen}
          onOpenChange={setFieldSelectOpen}
          value={selectedField ?? ALL_FIELDS_VALUE}
          onValueChange={handleFieldChange}
        >
          <SelectTrigger
            aria-label={fieldAriaLabel}
            title={`Search in ${selectedLabel}`}
            className="h-full w-[124px] shrink-0 rounded-none border-y-0 border-r-0 border-l border-border/60 px-2.5 focus:border-border/60 sm:w-[146px]"
          >
            <span className="min-w-0 truncate">
              <span className="text-muted-foreground">In: </span>
              {selectedLabel}
            </span>
          </SelectTrigger>
          <SelectContent align="end">
            <SelectItem value={ALL_FIELDS_VALUE}>{allFieldsLabel}</SelectItem>
            {fields.map((field) => (
              <SelectItem key={field.key} value={field.key}>
                {field.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    </form>
  );
}

export interface DataTableFilterPopoverProps<FilterKey extends string> {
  readonly fields: readonly DataTableFilterField<FilterKey>[];
  readonly values: DataTableFilterSelections<FilterKey>;
  readonly open: boolean;
  readonly selectedKey: FilterKey;
  readonly activeCount: number;
  readonly triggerLabel?: string;
  readonly validateValues?: (
    field: DataTableFilterField<FilterKey>,
    values: readonly string[],
  ) => boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onSelectField: (key: FilterKey) => void;
  readonly onApply: (selections: DataTableFilterSelections<FilterKey>) => void;
}

export function DataTableFilterPopover<FilterKey extends string>({
  fields,
  values,
  open,
  selectedKey,
  activeCount,
  triggerLabel = "Filters",
  validateValues,
  onOpenChange,
  onSelectField,
  onApply,
}: DataTableFilterPopoverProps<FilterKey>) {
  const selectedField =
    fields.find((field) => field.key === selectedKey) ?? fields[0];
  const selectionKey = JSON.stringify(
    fields.map((field) => [field.key, values[field.key] ?? []]),
  );

  function handleOpenChange(nextOpen: boolean) {
    if (nextOpen && selectedField === undefined && fields[0]) {
      onSelectField(fields[0].key);
    }
    onOpenChange(nextOpen);
  }

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant={activeCount > 0 ? "secondary" : "outline"}
          className="h-11 shrink-0 gap-2 px-3 md:h-8"
          disabled={fields.length === 0}
          aria-label={
            activeCount > 0
              ? `${triggerLabel}, ${String(activeCount)} applied`
              : triggerLabel
          }
        >
          <Filter className="h-3.5 w-3.5" aria-hidden="true" />
          <span>{triggerLabel}</span>
          {activeCount > 0 && (
            <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-primary/15 px-1 text-[10px] font-semibold text-primary">
              {String(activeCount)}
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        sideOffset={8}
        className="w-[min(36rem,calc(100vw-1.5rem))] overflow-hidden rounded-lg p-0"
      >
        <DataTableFilterPanel
          key={`${open ? "open" : "closed"}:${selectionKey}`}
          fields={fields}
          values={values}
          selectedField={selectedField}
          validateValues={validateValues}
          onSelectField={onSelectField}
          onCancel={() => onOpenChange(false)}
          onApply={(selections) => {
            onApply(selections);
            onOpenChange(false);
          }}
        />
      </PopoverContent>
    </Popover>
  );
}

function DataTableFilterPanel<FilterKey extends string>({
  fields,
  values,
  selectedField,
  validateValues,
  onSelectField,
  onCancel,
  onApply,
}: {
  readonly fields: readonly DataTableFilterField<FilterKey>[];
  readonly values: DataTableFilterSelections<FilterKey>;
  readonly selectedField: DataTableFilterField<FilterKey> | undefined;
  readonly validateValues?: (
    field: DataTableFilterField<FilterKey>,
    values: readonly string[],
  ) => boolean;
  readonly onSelectField: (key: FilterKey) => void;
  readonly onCancel: () => void;
  readonly onApply: (selections: DataTableFilterSelections<FilterKey>) => void;
}) {
  const [draftSelections, setDraftSelections] = useState<
    DataTableFilterSelections<FilterKey>
  >(() => ({ ...values }));

  return (
    <div className="grid min-h-[260px] grid-cols-[minmax(112px,0.85fr)_minmax(0,1.35fr)]">
      <div className="border-r border-border/70 bg-muted/20 p-1.5">
        {fields.map((field) => {
          const fieldValues = draftSelections[field.key] ?? [];
          const selectedValueCount =
            field.value_type === "date"
              ? Number(fieldValues.slice(1).some(Boolean))
              : fieldValues.length;
          const active = selectedValueCount > 0;
          const selected = field.key === selectedField?.key;
          return (
            <button
              key={field.key}
              type="button"
              className={cn(
                "flex min-h-11 w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring md:min-h-9",
                selected
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
              )}
              aria-pressed={selected}
              onClick={() => onSelectField(field.key)}
            >
              <span className="min-w-0 flex-1 break-words">{field.label}</span>
              {active && (
                <span
                  className="flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-primary/15 px-1 text-[10px] font-semibold text-primary"
                  aria-label={`${String(selectedValueCount)} selected`}
                >
                  {String(selectedValueCount)}
                </span>
              )}
              {selected && !active && (
                <ChevronRight
                  className="h-3.5 w-3.5 shrink-0"
                  aria-hidden="true"
                />
              )}
            </button>
          );
        })}
      </div>

      {selectedField ? (
        <DataTableFilterEditor
          key={`${selectedField.key}:${selectedField.date_modes?.join(",") ?? ""}`}
          field={selectedField}
          values={draftSelections[selectedField.key] ?? []}
          validateValues={validateValues}
          onValuesChange={(nextValues) =>
            setDraftSelections((current) => ({
              ...current,
              [selectedField.key]: nextValues,
            }))
          }
          onCancel={onCancel}
          onApply={() => onApply(draftSelections)}
        />
      ) : (
        <div />
      )}
    </div>
  );
}

function DataTableFilterEditor<FilterKey extends string>({
  field,
  values,
  validateValues,
  onValuesChange,
  onCancel,
  onApply,
}: {
  readonly field: DataTableFilterField<FilterKey>;
  readonly values: readonly string[];
  readonly validateValues?: (
    field: DataTableFilterField<FilterKey>,
    values: readonly string[],
  ) => boolean;
  readonly onValuesChange: (values: readonly string[]) => void;
  readonly onCancel: () => void;
  readonly onApply: () => void;
}) {
  if (field.value_type === "date") {
    return (
      <DataTableDateFilterEditor
        field={field}
        values={values}
        validateValues={validateValues}
        onValuesChange={onValuesChange}
        onCancel={onCancel}
        onApply={onApply}
      />
    );
  }

  const multiple = field.multiple === true;
  const allSelected =
    field.options.length > 0 && values.length === field.options.length;
  const partiallySelected = values.length > 0 && !allSelected;

  return (
    <div className="flex min-w-0 flex-col">
      <div className="border-b border-border/70 px-3 py-3">
        <div className="flex items-center justify-between gap-2">
          <p className="truncate text-xs font-semibold text-foreground">
            {field.label}{" "}
            <span className="font-normal text-muted-foreground">
              {field.operator}
            </span>
          </p>
          <span className="shrink-0 text-[10px] text-muted-foreground">
            {String(values.length)} selected
          </span>
        </div>
      </div>
      <div
        role="group"
        aria-label={`${field.label} values`}
        className="max-h-[250px] flex-1 space-y-1 overflow-y-auto p-2"
      >
        {multiple && (
          <button
            type="button"
            role="checkbox"
            aria-checked={partiallySelected ? "mixed" : allSelected}
            className={cn(
              "mb-2 flex min-h-11 w-full items-center gap-2 rounded-md border border-dashed px-3 py-2 text-left text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring md:min-h-9",
              allSelected || partiallySelected
                ? "border-primary/45 bg-primary/[0.07] text-foreground"
                : "border-border/80 bg-muted/20 text-muted-foreground hover:border-primary/35 hover:bg-muted/35 hover:text-foreground",
            )}
            onClick={() =>
              onValuesChange(
                allSelected ? [] : field.options.map((option) => option.value),
              )
            }
          >
            <span
              className={cn(
                "flex h-4 w-4 shrink-0 items-center justify-center rounded-[4px] border",
                allSelected || partiallySelected
                  ? "border-primary bg-primary text-primary-foreground"
                  : "border-muted-foreground/40",
              )}
              aria-hidden="true"
            >
              {allSelected ? (
                <Check className="h-3 w-3" />
              ) : partiallySelected ? (
                <Minus className="h-3 w-3" />
              ) : null}
            </span>
            <span>Select all</span>
            <span className="ml-auto text-[10px] font-normal text-muted-foreground">
              {String(field.options.length)} values
            </span>
          </button>
        )}
        {field.options.map((option) => {
          const checked = values.includes(option.value);
          return (
            <button
              key={option.value}
              type="button"
              role={multiple ? "checkbox" : undefined}
              aria-checked={multiple ? checked : undefined}
              aria-pressed={multiple ? undefined : checked}
              title={option.label === option.value ? undefined : option.value}
              className={cn(
                "flex min-h-11 w-full items-center gap-2 rounded-md border px-3 py-2 text-left text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring md:min-h-9",
                checked
                  ? "border-primary/40 bg-primary/10 text-foreground"
                  : "border-transparent text-muted-foreground hover:border-border hover:bg-accent/50 hover:text-foreground",
              )}
              onClick={() => {
                if (!multiple) {
                  onValuesChange([option.value]);
                  return;
                }
                onValuesChange(
                  checked
                    ? values.filter((value) => value !== option.value)
                    : [...values, option.value],
                );
              }}
            >
              <span
                className={cn(
                  "flex h-4 w-4 shrink-0 items-center justify-center border",
                  multiple ? "rounded-[4px]" : "rounded-full",
                  checked
                    ? "border-primary bg-primary text-primary-foreground"
                    : "border-muted-foreground/40",
                )}
                aria-hidden="true"
              >
                {checked && <Check className="h-3 w-3" />}
              </span>
              <span className="min-w-0 break-words">{option.label}</span>
            </button>
          );
        })}
      </div>
      <DataTableFilterActions
        hasValue={values.length > 0}
        onClear={() => onValuesChange([])}
        onCancel={onCancel}
        onApply={onApply}
      />
    </div>
  );
}

function DataTableDateFilterEditor<FilterKey extends string>({
  field,
  values,
  validateValues,
  onValuesChange,
  onCancel,
  onApply,
}: {
  readonly field: DataTableFilterField<FilterKey>;
  readonly values: readonly string[];
  readonly validateValues?: (
    field: DataTableFilterField<FilterKey>,
    values: readonly string[],
  ) => boolean;
  readonly onValuesChange: (values: readonly string[]) => void;
  readonly onCancel: () => void;
  readonly onApply: () => void;
}) {
  const dateModes =
    field.date_modes && field.date_modes.length > 0
      ? field.date_modes
      : DEFAULT_DATE_MODES;
  const initialMode =
    values[0] === "range" && dateModes.includes("range")
      ? "range"
      : (dateModes[0] ?? "dates");
  const [mode, setMode] = useState<"dates" | "range">(() => initialMode);
  const selectedDates = values[0] === "dates" ? values.slice(1) : [];
  const [from = "", to = ""] = values[0] === "range" ? values.slice(1) : [];
  const nextValues =
    mode === "dates" ? ["dates", ...selectedDates] : ["range", from, to];
  const hasValue =
    mode === "dates" ? selectedDates.length > 0 : Boolean(from || to);
  const valid =
    !hasValue ||
    (validateValues
      ? validateValues(field, nextValues)
      : isValidDateFilter(nextValues));

  function selectMode(nextMode: "dates" | "range") {
    if (nextMode === mode) return;
    setMode(nextMode);
    onValuesChange([nextMode]);
  }

  return (
    <div className="flex min-w-0 flex-col">
      <div className="border-b border-border/70 px-3 py-3">
        <div className="flex items-center justify-between gap-2">
          <p className="truncate text-xs font-semibold text-foreground">
            {field.label}{" "}
            <span className="font-normal text-muted-foreground">
              {mode === "dates" ? "on any selected date" : "between"}
            </span>
          </p>
          <span className="shrink-0 text-[10px] text-muted-foreground">
            {mode === "dates"
              ? `${String(selectedDates.length)} selected`
              : hasValue
                ? "Range selected"
                : "No range selected"}
          </span>
        </div>
      </div>

      <div className="flex max-h-[250px] flex-1 flex-col gap-4 overflow-y-auto p-3">
        {dateModes.length > 1 && (
          <div
            role="group"
            aria-label={`${field.label} date filter mode`}
            className="grid grid-cols-2 rounded-md border border-border/80 bg-muted/20 p-0.5"
          >
            {dateModes.includes("dates") && (
              <button
                type="button"
                aria-pressed={mode === "dates"}
                className={cn(
                  "flex min-h-9 items-center justify-center gap-1.5 rounded px-2 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                  mode === "dates"
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={() => selectMode("dates")}
              >
                <Calendar className="h-3.5 w-3.5" aria-hidden="true" />
                Specific dates
              </button>
            )}
            {dateModes.includes("range") && (
              <button
                type="button"
                aria-pressed={mode === "range"}
                className={cn(
                  "flex min-h-9 items-center justify-center gap-1.5 rounded px-2 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                  mode === "range"
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={() => selectMode("range")}
              >
                <CalendarRange className="h-3.5 w-3.5" aria-hidden="true" />
                Date range
              </button>
            )}
          </div>
        )}

        {mode === "dates" ? (
          <div className="space-y-1.5">
            <span className="text-[11px] font-medium text-muted-foreground">
              Dates
            </span>
            <DatePicker
              mode="multiple"
              values={selectedDates}
              maxSelections={field.max_values ?? 32}
              ariaLabel={`${field.label} on selected dates`}
              placeholder="Select dates"
              onValuesChange={(dates) => onValuesChange(["dates", ...dates])}
            />
            {selectedDates.length > 0 && (
              <div
                className="flex flex-wrap gap-1.5 pt-1"
                role="group"
                aria-label={`Selected ${field.label.toLocaleLowerCase()} dates`}
              >
                {selectedDates.map((date) => (
                  <button
                    key={date}
                    type="button"
                    className="flex min-h-8 items-center gap-1.5 rounded-md border border-border/80 bg-muted/25 px-2 text-[11px] text-foreground outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
                    aria-label={`Remove selected date ${date}`}
                    onClick={() =>
                      onValuesChange([
                        "dates",
                        ...selectedDates.filter((value) => value !== date),
                      ])
                    }
                  >
                    <span>{selectedDateLabel(date)}</span>
                    <X
                      className="h-3 w-3 text-muted-foreground"
                      aria-hidden="true"
                    />
                  </button>
                ))}
              </div>
            )}
          </div>
        ) : (
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <span className="text-[11px] font-medium text-muted-foreground">
                From
              </span>
              <DatePicker
                value={from || null}
                ariaLabel={`${field.label} from date`}
                placeholder="Any date"
                onChange={(value) =>
                  onValuesChange(["range", value ?? "", value || to ? to : ""])
                }
              />
            </div>
            <div className="space-y-1.5">
              <span className="text-[11px] font-medium text-muted-foreground">
                To
              </span>
              <DatePicker
                value={to || null}
                minDate={from || undefined}
                ariaLabel={`${field.label} to date`}
                placeholder="Any date"
                onChange={(value) =>
                  onValuesChange(["range", from, value ?? ""])
                }
              />
            </div>
          </div>
        )}

        {!valid && (
          <p role="alert" className="text-xs text-destructive">
            The end date must be on or after the start date.
          </p>
        )}
      </div>

      <DataTableFilterActions
        hasValue={hasValue}
        disabled={!valid}
        onClear={() => onValuesChange([])}
        onCancel={onCancel}
        onApply={onApply}
      />
    </div>
  );
}

function DataTableFilterActions({
  hasValue,
  disabled = false,
  onClear,
  onCancel,
  onApply,
}: {
  readonly hasValue: boolean;
  readonly disabled?: boolean;
  readonly onClear: () => void;
  readonly onCancel: () => void;
  readonly onApply: () => void;
}) {
  return (
    <div className="flex flex-col gap-2 border-t border-border/70 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
      <div className="min-h-7">
        {hasValue && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="px-2 text-muted-foreground"
            onClick={onClear}
          >
            Clear all
          </Button>
        )}
      </div>
      <div className="flex items-center gap-2">
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          type="button"
          variant="primary"
          disabled={disabled}
          onClick={onApply}
        >
          Apply filters
        </Button>
      </div>
    </div>
  );
}

function selectedDateLabel(value: string): string {
  const [year, month, day] = value.split("-").map(Number);
  if (!year || !month || !day) return value;
  return FILTER_DATE_FORMATTER.format(new Date(Date.UTC(year, month - 1, day)));
}

function isCalendarDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(5, 7));
  const day = Number(value.slice(8, 10));
  const date = new Date(0);
  date.setUTCHours(0, 0, 0, 0);
  date.setUTCFullYear(year, month - 1, day);
  return (
    date.getUTCFullYear() === year &&
    date.getUTCMonth() === month - 1 &&
    date.getUTCDate() === day
  );
}

function isValidDateFilter(values: readonly string[]): boolean {
  const [mode, ...dateValues] = values;
  if (mode === "dates") {
    return dateValues.length > 0 && dateValues.every(isCalendarDate);
  }
  if (mode !== "range") return false;
  const [from = "", to = "", ...extra] = dateValues;
  return (
    extra.length === 0 &&
    Boolean(from || to) &&
    (!from || isCalendarDate(from)) &&
    (!to || isCalendarDate(to)) &&
    (!from || !to || from <= to)
  );
}

export interface DataTableFilterChipsProps<
  SearchKey extends string,
  FilterKey extends string,
> {
  readonly search: string;
  readonly searchFields: readonly DataTableSearchField<SearchKey>[];
  readonly searchFilters: readonly DataTableSearchGroup<SearchKey>[];
  readonly filters: readonly AppliedDataTableFilter<FilterKey>[];
  readonly allFieldsLabel?: string;
  readonly ariaLabel?: string;
  readonly onEditSearch: () => void;
  readonly onRemoveSearch: () => void;
  readonly onEditSearchValue: (field: SearchKey, value: string) => void;
  readonly onRemoveSearchValue: (field: SearchKey, value: string) => void;
  readonly onEdit: (key: FilterKey) => void;
  readonly onRemove: (key: FilterKey) => void;
  readonly onClear: () => void;
}

export function DataTableFilterChips<
  SearchKey extends string,
  FilterKey extends string,
>({
  search,
  searchFields,
  searchFilters,
  filters,
  allFieldsLabel = "All fields",
  ariaLabel = "Applied filters",
  onEditSearch,
  onRemoveSearch,
  onEditSearchValue,
  onRemoveSearchValue,
  onEdit,
  onRemove,
  onClear,
}: DataTableFilterChipsProps<SearchKey, FilterKey>) {
  if (!search && searchFilters.length === 0 && filters.length === 0)
    return null;

  const visibleSearch =
    search.length > 48 ? `${search.slice(0, 48)}...` : search;
  const searchLabels = new Map(
    searchFields.map((field) => [field.key, field.label] as const),
  );

  return (
    <div className="flex flex-wrap items-center gap-1.5" aria-label={ariaLabel}>
      {search && (
        <div
          role="group"
          aria-label={`${allFieldsLabel} search`}
          className="flex min-h-11 max-w-full items-stretch overflow-hidden rounded-md border border-border/80 bg-muted/25 text-xs md:min-h-9"
        >
          <button
            type="button"
            className="min-w-0 px-2.5 py-1.5 text-left outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
            aria-label={`Edit ${allFieldsLabel} search: contains ${search}`}
            title={search}
            onClick={onEditSearch}
          >
            <span className="font-medium text-foreground">
              {allFieldsLabel}
            </span>{" "}
            <span className="text-muted-foreground">contains</span>{" "}
            <span className="break-all font-medium text-foreground">
              &quot;{visibleSearch}&quot;
            </span>
          </button>
          <button
            type="button"
            className="flex w-11 shrink-0 items-center justify-center border-l border-border/70 text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring md:w-9"
            aria-label={`Remove ${allFieldsLabel} search`}
            title={`Remove ${allFieldsLabel} search`}
            onClick={onRemoveSearch}
          >
            <X className="h-3.5 w-3.5" aria-hidden="true" />
          </button>
        </div>
      )}
      {searchFilters.map((searchFilter) => {
        const label =
          searchLabels.get(searchFilter.field) ?? searchFilter.field;
        return (
          <div
            key={searchFilter.field}
            role="group"
            aria-label={`${label} search, matches any term`}
            className="flex min-h-11 max-w-full flex-wrap items-stretch overflow-hidden rounded-md border border-border/80 bg-muted/25 text-xs md:min-h-9"
          >
            <span className="flex items-center px-2.5 py-1.5">
              <span className="font-medium text-foreground">{label}</span>{" "}
              <span className="ml-1 text-muted-foreground">contains</span>
            </span>
            {searchFilter.values.map((value, valueIndex) => {
              const visibleValue =
                value.length > 36 ? `${value.slice(0, 36)}...` : value;
              return (
                <Fragment key={value.toLocaleLowerCase()}>
                  {valueIndex > 0 && (
                    <span className="flex items-center border-l border-border/70 px-2 text-[10px] font-semibold text-muted-foreground">
                      OR
                    </span>
                  )}
                  <div className="flex min-w-0 items-stretch border-l border-border/70">
                    <button
                      type="button"
                      className="min-w-0 px-2 py-1.5 text-left font-medium text-foreground outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                      aria-label={`Edit ${label} search: contains ${value}`}
                      title={value}
                      onClick={() =>
                        onEditSearchValue(searchFilter.field, value)
                      }
                    >
                      <span className="break-all">
                        &quot;{visibleValue}&quot;
                      </span>
                    </button>
                    <button
                      type="button"
                      className="flex w-11 shrink-0 items-center justify-center text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring md:w-8"
                      aria-label={`Remove ${label} search value: ${value}`}
                      title={`Remove ${label} search value`}
                      onClick={() =>
                        onRemoveSearchValue(searchFilter.field, value)
                      }
                    >
                      <X className="h-3.5 w-3.5" aria-hidden="true" />
                    </button>
                  </div>
                </Fragment>
              );
            })}
          </div>
        );
      })}
      {filters.map(
        ({ field, values, valueLabels, operatorLabel, valueSummary }) => {
          const resolvedOperatorLabel =
            operatorLabel ??
            (values.length > 1
              ? field.operator === "includes"
                ? "includes any of"
                : "is any of"
              : field.operator);
          const hiddenValueCount = Math.max(0, valueLabels.length - 2);
          const visibleValueSummary =
            valueSummary ??
            `${valueLabels.slice(0, 2).join(", ")}${
              hiddenValueCount > 0 ? ` +${String(hiddenValueCount)} more` : ""
            }`;
          const fullValueSummary = valueSummary ?? valueLabels.join(", ");
          return (
            <div
              key={field.key}
              className="flex min-h-11 max-w-full items-stretch overflow-hidden rounded-md border border-border/80 bg-muted/25 text-xs md:min-h-9"
            >
              <button
                type="button"
                className="min-w-0 px-2.5 py-1.5 text-left outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                aria-label={`Edit ${field.label} filter: ${resolvedOperatorLabel} ${fullValueSummary}`}
                title={fullValueSummary}
                onClick={() => onEdit(field.key)}
              >
                <span className="font-medium text-foreground">
                  {field.label}
                </span>{" "}
                <span className="text-muted-foreground">
                  {resolvedOperatorLabel}
                </span>{" "}
                <span className="break-all font-medium text-foreground">
                  {visibleValueSummary}
                </span>
              </button>
              <button
                type="button"
                className="flex w-11 shrink-0 items-center justify-center border-l border-border/70 text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring md:w-9"
                aria-label={`Remove ${field.label} filter`}
                title={`Remove ${field.label} filter`}
                onClick={() => onRemove(field.key)}
              >
                <X className="h-3.5 w-3.5" aria-hidden="true" />
              </button>
            </div>
          );
        },
      )}
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-11 px-2 text-xs text-muted-foreground md:h-9"
        onClick={onClear}
      >
        Clear filters
      </Button>
    </div>
  );
}

export function DataTableControls({
  search,
  filter,
  status,
  chips,
}: {
  readonly search: ReactNode;
  readonly filter: ReactNode;
  readonly status?: ReactNode;
  readonly chips?: ReactNode;
}) {
  return (
    <div className="space-y-2 border-b border-border/60 p-3">
      <div className="flex flex-wrap items-center gap-2">
        {search}
        <div className="ml-auto flex shrink-0 items-center">{filter}</div>
        {status}
      </div>
      {chips}
    </div>
  );
}
