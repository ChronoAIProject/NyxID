import { localTime } from "@/lib/automation-time";
import { DatePicker } from "@/components/ui/date-picker";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { GroupedInput, GroupedRow } from "@/components/ui/grouped-list";

export function AutomationDateTime({
  value,
  onChange,
  label,
  optional = false,
  timezone,
  grouped = false,
  errorId,
  invalid,
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly label: string;
  readonly optional?: boolean;
  readonly timezone: string;
  readonly grouped?: boolean;
  readonly errorId?: string;
  readonly invalid?: boolean;
}) {
  const [date = "", time = "09:00"] = value.split("T");
  const TimeInput = grouped ? GroupedInput : Input;
  const controls = (
    <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
      <DatePicker
        grouped={grouped}
        describedBy={errorId}
        invalid={invalid}
        ariaLabel={`${label} date`}
        value={date || null}
        onChange={(next) => onChange(next ? `${next}T${time}` : "")}
      />
      <TimeInput
        type="time"
        aria-label={`${label} time`}
        aria-describedby={errorId}
        aria-invalid={invalid}
        value={time}
        onChange={(event) =>
          onChange(
            `${date || localTime(new Date().toISOString(), timezone).slice(0, 10)}T${event.target.value}`,
          )
        }
        className="w-28"
      />
      {optional && value && (
        <Button
          type="button"
          variant="ghost"
          aria-label={`Clear ${label}`}
          onClick={() => onChange("")}
        >
          Clear
        </Button>
      )}
    </div>
  );
  if (grouped)
    return (
      <GroupedRow
        title={`${label}${optional ? " (optional)" : ""}`}
        accessory={controls}
      />
    );
  return (
    <fieldset className="space-y-1 text-12">
      <legend>
        {label}
        {optional ? " (optional)" : ""}
      </legend>
      <div className="flex gap-2">
        <DatePicker
          ariaLabel={`${label} date`}
          value={date || null}
          onChange={(next) => onChange(next ? `${next}T${time}` : "")}
        />
        <Input
          type="time"
          aria-label={`${label} time`}
          value={time}
          onChange={(event) =>
            onChange(
              `${date || localTime(new Date().toISOString(), timezone).slice(0, 10)}T${event.target.value}`,
            )
          }
          className="w-28"
        />
        {optional && value && (
          <Button
            type="button"
            variant="ghost"
            aria-label={`Clear ${label}`}
            onClick={() => onChange("")}
          >
            Clear
          </Button>
        )}
      </div>
    </fieldset>
  );
}
