import { useState, useMemo, useCallback } from "react";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Calendar, ChevronLeft, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";

const DAYS = ["S", "M", "T", "W", "T", "F", "S"] as const;
const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
] as const;

function getDaysInMonth(year: number, month: number): number {
  return new Date(year, month + 1, 0).getDate();
}

function getFirstDayOfMonth(year: number, month: number): number {
  return new Date(year, month, 1).getDay();
}

function toDateString(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function parseDate(value: string): Date | null {
  if (!value) return null;
  const [y, m, d] = value.split("-").map(Number);
  if (!y || !m || !d) return null;
  return new Date(y, m - 1, d);
}

interface CommonDatePickerProps {
  readonly minDate?: string;
  readonly placeholder?: string;
  readonly ariaLabel?: string;
  readonly disabled?: boolean;
}

type DatePickerProps = CommonDatePickerProps &
  (
    | {
        readonly mode?: "single";
        readonly value: string | null;
        readonly onChange: (value: string | null) => void;
      }
    | {
        readonly mode: "multiple";
        readonly values: readonly string[];
        readonly onValuesChange: (values: readonly string[]) => void;
        readonly maxSelections?: number;
      }
  );

export function DatePicker(props: DatePickerProps) {
  const {
    minDate,
    placeholder = "Select date",
    ariaLabel,
    disabled = false,
  } = props;
  const multiple = props.mode === "multiple";
  const values = multiple ? props.values : props.value ? [props.value] : [];
  const maxSelections = multiple ? (props.maxSelections ?? 32) : 1;
  const [open, setOpen] = useState(false);
  const selected = values[0] ? parseDate(values[0]) : null;
  const selectedValues = new Set(values);
  const min = useMemo(() => (minDate ? parseDate(minDate) : null), [minDate]);

  const today = useMemo(() => new Date(), []);
  const [viewYear, setViewYear] = useState(
    () => selected?.getFullYear() ?? today.getFullYear(),
  );
  const [viewMonth, setViewMonth] = useState(
    () => selected?.getMonth() ?? today.getMonth(),
  );

  const prevMonth = useCallback(() => {
    setViewMonth((m) => {
      if (m === 0) {
        setViewYear((y) => y - 1);
        return 11;
      }
      return m - 1;
    });
  }, []);

  const nextMonth = useCallback(() => {
    setViewMonth((m) => {
      if (m === 11) {
        setViewYear((y) => y + 1);
        return 0;
      }
      return m + 1;
    });
  }, []);

  const daysInMonth = getDaysInMonth(viewYear, viewMonth);
  const firstDay = getFirstDayOfMonth(viewYear, viewMonth);
  const prevMonthDays = getDaysInMonth(
    viewMonth === 0 ? viewYear - 1 : viewYear,
    viewMonth === 0 ? 11 : viewMonth - 1,
  );

  const cells: Array<{
    day: number;
    current: boolean;
    disabled: boolean;
    date: Date;
  }> = useMemo(() => {
    const result: Array<{
      day: number;
      current: boolean;
      disabled: boolean;
      date: Date;
    }> = [];

    for (let i = firstDay - 1; i >= 0; i--) {
      const d = prevMonthDays - i;
      const date = new Date(
        viewMonth === 0 ? viewYear - 1 : viewYear,
        viewMonth === 0 ? 11 : viewMonth - 1,
        d,
      );
      result.push({ day: d, current: false, disabled: true, date });
    }

    for (let d = 1; d <= daysInMonth; d++) {
      const date = new Date(viewYear, viewMonth, d);
      const isBeforeMin =
        min !== null &&
        date < new Date(min.getFullYear(), min.getMonth(), min.getDate());
      result.push({ day: d, current: true, disabled: isBeforeMin, date });
    }

    const remaining = 42 - result.length;
    for (let d = 1; d <= remaining; d++) {
      const date = new Date(
        viewMonth === 11 ? viewYear + 1 : viewYear,
        viewMonth === 11 ? 0 : viewMonth + 1,
        d,
      );
      result.push({ day: d, current: false, disabled: true, date });
    }

    return result;
  }, [viewYear, viewMonth, firstDay, prevMonthDays, daysInMonth, min]);

  function isSelected(date: Date): boolean {
    return selectedValues.has(toDateString(date));
  }

  function isToday(date: Date): boolean {
    return (
      date.getFullYear() === today.getFullYear() &&
      date.getMonth() === today.getMonth() &&
      date.getDate() === today.getDate()
    );
  }

  function selectDay(date: Date) {
    const nextDate = toDateString(date);
    if (multiple) {
      props.onValuesChange(
        selectedValues.has(nextDate)
          ? values.filter((value) => value !== nextDate)
          : [...values, nextDate].sort(),
      );
      return;
    }
    props.onChange(nextDate);
    setOpen(false);
  }

  const displayValue =
    multiple && values.length > 1
      ? `${String(values.length)} dates selected`
      : selected
        ? `${MONTHS[selected.getMonth()]} ${selected.getDate()}, ${selected.getFullYear()}`
        : null;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          aria-label={ariaLabel}
          className={cn(
            "flex h-8 w-full items-center justify-between rounded-lg border border-input bg-transparent px-3 text-[12px] transition-colors",
            "hover:border-white/[0.15] focus-visible:outline-none focus-visible:border-white/[0.15]",
            "disabled:cursor-not-allowed disabled:opacity-50",
            displayValue ? "text-foreground" : "text-text-tertiary",
          )}
        >
          <span>{displayValue ?? placeholder}</span>
          <Calendar className="h-3.5 w-3.5 text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-[280px] p-3" align="start">
        <div className="space-y-3">
          {/* Header */}
          <div className="flex items-center justify-between">
            <button
              type="button"
              aria-label="Previous month"
              onClick={prevMonth}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-white/[0.06] hover:text-foreground"
            >
              <ChevronLeft className="h-4 w-4" aria-hidden="true" />
            </button>
            <span className="text-[12px] font-medium">
              {MONTHS[viewMonth]} {viewYear}
            </span>
            <button
              type="button"
              aria-label="Next month"
              onClick={nextMonth}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-white/[0.06] hover:text-foreground"
            >
              <ChevronRight className="h-4 w-4" aria-hidden="true" />
            </button>
          </div>

          {/* Day labels */}
          <div className="grid grid-cols-7 text-center">
            {DAYS.map((d, i) => (
              <span
                key={i}
                className="text-[10px] font-semibold uppercase tracking-[1px] text-text-tertiary py-1"
              >
                {d}
              </span>
            ))}
          </div>

          {/* Day grid */}
          <div className="grid grid-cols-7">
            {cells.map((cell, i) => {
              const sel = isSelected(cell.date);
              const tod = isToday(cell.date);
              const selectionLimitReached =
                multiple && !sel && values.length >= maxSelections;
              return (
                <button
                  key={i}
                  type="button"
                  disabled={cell.disabled || selectionLimitReached}
                  aria-label={toDateString(cell.date)}
                  aria-pressed={sel}
                  onClick={() => selectDay(cell.date)}
                  className={cn(
                    "flex h-8 w-full items-center justify-center rounded-md text-[12px] transition-colors",
                    !cell.current && "text-text-tertiary/40",
                    cell.current &&
                      !sel &&
                      !cell.disabled &&
                      "text-foreground hover:bg-white/[0.06]",
                    (cell.disabled || selectionLimitReached) &&
                      "cursor-not-allowed opacity-30",
                    sel && "bg-primary text-primary-foreground font-medium",
                    tod && !sel && "font-medium text-primary",
                  )}
                >
                  {cell.day}
                </button>
              );
            })}
          </div>

          {/* Footer */}
          <div className="flex items-center justify-between border-t border-border/50 pt-2">
            <Button
              type="button"
              variant="ghost"
              className="h-7 text-[11px]"
              disabled={values.length === 0}
              onClick={() => {
                if (multiple) {
                  props.onValuesChange([]);
                } else {
                  props.onChange(null);
                  setOpen(false);
                }
              }}
            >
              {multiple ? "Clear all" : "Clear"}
            </Button>
            <div className="flex items-center gap-1">
              <Button
                type="button"
                variant="ghost"
                className="h-7 text-[11px]"
                onClick={() => {
                  const t = new Date();
                  const todayValue = toDateString(t);
                  setViewYear(t.getFullYear());
                  setViewMonth(t.getMonth());
                  if (multiple) {
                    if (
                      !selectedValues.has(todayValue) &&
                      values.length < maxSelections
                    ) {
                      props.onValuesChange([...values, todayValue].sort());
                    }
                  } else {
                    props.onChange(todayValue);
                    setOpen(false);
                  }
                }}
              >
                Today
              </Button>
              {multiple && (
                <Button
                  type="button"
                  variant="ghost"
                  className="h-7 text-[11px]"
                  onClick={() => setOpen(false)}
                >
                  Done
                </Button>
              )}
            </div>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
