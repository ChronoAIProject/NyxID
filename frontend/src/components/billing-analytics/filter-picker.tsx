import { useState, type ReactNode } from "react";
import { Check, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Skeleton } from "@/components/ui/skeleton";

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
 * Multi-select popover shared by admin usage analytics and personal billing
 * usage: check rows, search, and a draft committed only by Apply. Callers own
 * option loading (and therefore which APIs are called) and the trigger.
 */
export function FilterPicker({
  label,
  trigger,
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
  trigger: ReactNode;
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
  const selected = (option: FilterPickerOption) =>
    draft.some((value) => matches(value, option));
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (next) {
          setDraft([...values]);
          onSearchChange("");
        }
        onOpenChange(next);
      }}
    >
      <PopoverTrigger asChild>{trigger}</PopoverTrigger>
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
