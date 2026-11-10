import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

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
  const { t } = useUiTranslation();
  const [date = "", time = "09:00"] = value.split("T");
  return (
    <fieldset className="space-y-1 text-12">
      <legend>
        {label}
        {optional ? t(" (optional)") : ""}
      </legend>
      <div className="flex gap-2">
        <DatePicker
          ariaLabel={translateUi("{{value0}} date", { value0: label })}
          value={date || null}
          onChange={(next) => onChange(next ? `${next}T${time}` : "")}
        />
        <Input
          type="time"
          aria-label={t("{{label}} time", { label: label })}
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
            aria-label={t("Clear {{label}}", { label: label })}
            onClick={() => onChange("")}
          >{t("Clear")}</Button>
        )}
      </div>
    </fieldset>
  );
}
