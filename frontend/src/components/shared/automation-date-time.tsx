import { localTime } from "@/lib/automation-time";
import { DatePicker } from "@/components/ui/date-picker";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";

export function AutomationDateTime({
  value,
  onChange,
  label,
  optional = false,
  timezone,
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly label: string;
  readonly optional?: boolean;
  readonly timezone: string;
}) {
  const [date = "", time = "09:00"] = value.split("T");
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
