import { useId, useState, type ReactNode } from "react";
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

export interface FilterPickerOption {
  readonly id: string;
  readonly label: string;
  readonly detail?: string;
}

export type FilterPickerOptions =
  | { readonly status: "pending" }
  | { readonly status: "error"; readonly onRetry: () => void }
  | {
      readonly status: "success";
      readonly options: readonly FilterPickerOption[];
      readonly total: number;
    };

/**
 * Multi-select filter shared by admin usage analytics and personal billing
 * usage: the outline trigger, check rows, search, and a draft committed only
 * by Apply. Callers own option loading, so each surface calls only its own
 * APIs, but never the trigger or the draft lifecycle.
 */
export function FilterPicker({
  label,
  description,
  disabled = false,
  values,
  onChange,
  open,
  onOpenChange,
  search,
  onSearchChange,
  searchPlaceholder,
  options,
  matches = (value, option) => value === option.id,
  limit,
}: {
  label: string;
  /** Optional caption under the popover title. */
  description?: string;
  disabled?: boolean;
  values: readonly string[];
  onChange: (values: string[]) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  search: string;
  onSearchChange: (search: string) => void;
  searchPlaceholder: string;
  options: FilterPickerOptions;
  matches?: (value: string, option: FilterPickerOption) => boolean;
  limit?: number;
}) {
  const [draft, setDraft] = useState<string[]>([...values]);
  // Every opening starts from the applied selection, however it was opened
  // (trigger, keyboard, or a caller such as a filter chip).
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setDraft([...values]);
  }
  const selected = (option: FilterPickerOption) =>
    draft.some((value) => matches(value, option));
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (next) onSearchChange("");
        onOpenChange(next);
      }}
    >
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          aria-label={`Filter ${label.toLowerCase()}`}
          className="justify-between"
          disabled={disabled}
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
        {description && (
          <p className="-mt-1.5 text-[11px] text-muted-foreground">
            {description}
          </p>
        )}
        <div className="relative">
          <Search className="absolute left-2.5 top-2 size-3.5 text-muted-foreground" />
          <Input
            value={search}
            onChange={(e) => onSearchChange(e.target.value)}
            placeholder={searchPlaceholder}
            aria-label={`Search ${label.toLowerCase()}`}
            className="pl-8"
          />
        </div>
        {options.status === "pending" ? (
          <Skeleton className="h-24" />
        ) : options.status === "error" ? (
          <ErrorBanner
            message={`Could not load ${label.toLowerCase()}.`}
            onRetry={options.onRetry}
          />
        ) : (
          <div className="max-h-64 space-y-1 overflow-auto">
            {options.options.map((option) => (
              <Button
                key={option.id}
                variant="ghost"
                className="h-auto w-full justify-start py-2 text-left"
                disabled={
                  !selected(option) &&
                  limit !== undefined &&
                  draft.length >= limit
                }
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
            {options.options.length === 0 && (
              <p className="p-2 text-[12px] text-muted-foreground">
                No matches.
              </p>
            )}
            {options.total > options.options.length && (
              <p className="text-[10px] text-muted-foreground">
                Search to narrow {options.total.toLocaleString()} matches.
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

/**
 * The usage filter card: pickers on the left, a period select on the right,
 * applied-filter chips and any extra rows below.
 */
export function FilterCard({
  pickers,
  aside,
  children,
}: {
  pickers: ReactNode;
  aside: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="space-y-3 rounded-xl border border-border/50 bg-card px-4 py-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-2">{pickers}</div>
        <div className="min-w-40">{aside}</div>
      </div>
      {children}
    </div>
  );
}

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
