import { useState, type ReactNode } from "react";
import { Check, ChevronDown, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";

export interface BillingMultiSelectOption {
  readonly id: string;
  readonly label: string;
  readonly detail?: string;
}

export interface BillingMultiSelectProps {
  label: string;
  itemLabel: string;
  values: string[];
  onChange: (values: string[]) => void;
  options: BillingMultiSelectOption[];
  children?: ReactNode;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  emptyLabel?: string;
  description?: string;
  disabled?: boolean;
  limit?: number;
  capitalize?: boolean;
  className?: string;
}

export function BillingMultiSelect({
  label,
  itemLabel,
  emptyLabel = "None",
  description,
  disabled = false,
  limit,
  capitalize = false,
  className,
  values,
  onChange,
  options,
  children,
  open: controlledOpen,
  onOpenChange,
}: BillingMultiSelectProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const [search, setSearch] = useState("");
  const open = controlledOpen ?? internalOpen;
  const setOpen = (next: boolean) => {
    setInternalOpen(next);
    setSearch("");
    onOpenChange?.(next);
  };
  const needle = search.trim().toLowerCase();
  const filtered = options.filter((option) =>
    `${option.label} ${option.detail ?? ""}`.toLowerCase().includes(needle),
  );
  const nameOf = (value: string) =>
    options.find((option) => option.id === value)?.label ?? value;
  const remove = (value: string) =>
    onChange(values.filter((id) => id !== value));

  return (
    <div className={cn("billing-filter-controls", className)}>
      <div className="billing-filter-control-row">
        <Popover open={open} onOpenChange={setOpen}>
          <PopoverTrigger asChild>
            <Button
              variant="outline"
              aria-label={`Filter ${label.toLowerCase()}`}
              disabled={disabled}
              className="min-w-40 justify-between gap-6"
            >
              <span>{label}</span>
              <span className="ml-auto text-muted-foreground">
                {values.length ? `${values.length} selected` : emptyLabel}
              </span>
              <ChevronDown className="size-3" aria-hidden="true" />
            </Button>
          </PopoverTrigger>
          <PopoverContent
            align="start"
            aria-label={`Choose ${label.toLowerCase()}`}
            className="w-72 space-y-3 p-3"
          >
            {description && (
              <p className="text-11 text-muted-foreground">{description}</p>
            )}
            <Input
              aria-label={`Search ${label.toLowerCase()}`}
              placeholder={`Search ${label.toLowerCase()}…`}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
            <div
              className="max-h-64 space-y-1 overflow-y-auto"
              role="group"
              aria-label={label}
            >
              {filtered.map((option) => {
                const checked = values.includes(option.id);
                return (
                  <button
                    key={option.id}
                    type="button"
                    role="checkbox"
                    aria-checked={checked}
                    disabled={
                      !checked && limit !== undefined && values.length >= limit
                    }
                    className={cn(
                      "flex min-h-9 w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-12 text-foreground outline-none transition-colors hover:bg-overlay-strong focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
                      checked && "bg-overlay",
                    )}
                    onClick={() =>
                      checked
                        ? remove(option.id)
                        : onChange([...values, option.id])
                    }
                  >
                    <span
                      aria-hidden="true"
                      className={cn(
                        "flex size-4 shrink-0 items-center justify-center rounded-[4px] border",
                        checked
                          ? "border-primary bg-primary text-primary-foreground"
                          : "border-input",
                      )}
                    >
                      {checked && <Check className="size-3" />}
                    </span>
                    <span className="min-w-0">
                      <span className={cn("block", capitalize && "capitalize")}>
                        {option.label}
                      </span>
                      {option.detail && (
                        <span className="block text-10 text-muted-foreground">
                          {option.detail}
                        </span>
                      )}
                    </span>
                  </button>
                );
              })}
              {!filtered.length && (
                <p className="p-2 text-12 text-muted-foreground">
                  No matching {label.toLowerCase()}.
                </p>
              )}
            </div>
            <div className="flex items-center justify-between border-t border-border pt-2">
              <Button
                variant="ghost"
                size="sm"
                disabled={!values.length}
                onClick={() => onChange([])}
              >
                Clear
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setOpen(false)}
              >
                Done
              </Button>
            </div>
          </PopoverContent>
        </Popover>
        {children}
      </div>
      {values.length > 0 && (
        <div
          className="flex flex-wrap items-center gap-1.5"
          aria-label={`Selected ${label.toLowerCase()}`}
        >
          {values.map((value) => (
            <div
              key={value}
              className="flex min-w-0 max-w-full items-center gap-2 rounded-lg border border-border px-2.5 py-1 text-11"
            >
              <button
                type="button"
                className="min-w-0 rounded text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
                aria-label={`Edit ${itemLabel.toLowerCase()} filter: ${nameOf(value)}`}
                onClick={() => setOpen(true)}
              >
                <span className="text-muted-foreground">{itemLabel}:</span>{" "}
                <span className={cn("break-words", capitalize && "capitalize")}>
                  {nameOf(value)}
                </span>
              </button>
              <button
                type="button"
                className="shrink-0 rounded p-1 text-muted-foreground hover:bg-overlay-strong hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                aria-label={`Remove ${itemLabel.toLowerCase()}: ${nameOf(value)}`}
                onClick={() => remove(value)}
              >
                <X className="size-3" aria-hidden="true" />
              </button>
            </div>
          ))}
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground"
            onClick={() => onChange([])}
          >
            Clear filters
          </Button>
        </div>
      )}
    </div>
  );
}
