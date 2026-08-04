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

interface DatePickerProps {
  readonly value: string | null;
  readonly onChange: (value: string | null) => void;
  readonly minDate?: string;
  readonly placeholder?: string;
  readonly disabled?: boolean;
}

export function DatePicker({
  value,
  onChange,
  minDate,
  placeholder = "Select date",
  disabled = false,
}: DatePickerProps) {
  const [open, setOpen] = useState(false);
  const selected = useMemo(() => (value ? parseDate(value) : null), [value]);
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
        min !== null && date < new Date(min.getFullYear(), min.getMonth(), min.getDate());
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
    if (!selected) return false;
    return (
      date.getFullYear() === selected.getFullYear() &&
      date.getMonth() === selected.getMonth() &&
      date.getDate() === selected.getDate()
    );
  }

  function isToday(date: Date): boolean {
    return (
      date.getFullYear() === today.getFullYear() &&
      date.getMonth() === today.getMonth() &&
      date.getDate() === today.getDate()
    );
  }

  function selectDay(date: Date) {
    onChange(toDateString(date));
    setOpen(false);
  }

  const displayValue = selected
    ? `${MONTHS[selected.getMonth()]} ${selected.getDate()}, ${selected.getFullYear()}`
    : null;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
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
              onClick={prevMonth}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-white/[0.06] hover:text-foreground"
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <span className="text-[12px] font-medium">
              {MONTHS[viewMonth]} {viewYear}
            </span>
            <button
              type="button"
              onClick={nextMonth}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-white/[0.06] hover:text-foreground"
            >
              <ChevronRight className="h-4 w-4" />
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
              return (
                <button
                  key={i}
                  type="button"
                  disabled={cell.disabled}
                  onClick={() => selectDay(cell.date)}
                  className={cn(
                    "flex h-8 w-full items-center justify-center rounded-md text-[12px] transition-colors",
                    !cell.current && "text-text-tertiary/40",
                    cell.current &&
                      !sel &&
                      !cell.disabled &&
                      "text-foreground hover:bg-white/[0.06]",
                    cell.disabled && "cursor-not-allowed opacity-30",
                    sel &&
                      "bg-primary text-primary-foreground font-medium",
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
              onClick={() => {
                onChange(null);
                setOpen(false);
              }}
            >
              Clear
            </Button>
            <Button
              type="button"
              variant="ghost"
              className="h-7 text-[11px]"
              onClick={() => {
                const t = new Date();
                setViewYear(t.getFullYear());
                setViewMonth(t.getMonth());
                onChange(toDateString(t));
                setOpen(false);
              }}
            >
              Today
            </Button>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
