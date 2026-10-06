import { useState } from "react";
import { Check, ChevronsUpDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { browserTimezone } from "@/lib/automation-time";

export function TimezoneSelect({
  value,
  onChange,
  label = "Timezone",
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly label?: string;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const suggested = browserTimezone();
  const zones = [
    ...new Set([
      suggested,
      "UTC",
      value,
      ...Intl.supportedValuesOf("timeZone"),
    ]),
  ].filter(Boolean);
  return (
    <div className="space-y-1 text-12">
      <span>{label}</span>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            role="combobox"
            aria-label={label}
            aria-expanded={open}
            className="w-full justify-between"
          >
            {value || "Choose a timezone"}
            <ChevronsUpDown className="size-3.5 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-72 p-2" align="start">
          <Input
            aria-label="Search timezones"
            placeholder="Search timezones…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
          <div
            role="listbox"
            aria-label="Timezones"
            className="mt-2 max-h-64 overflow-y-auto"
          >
            {zones
              .filter((zone) =>
                zone.toLowerCase().includes(search.toLowerCase()),
              )
              .map((zone) => (
                <button
                  key={zone}
                  type="button"
                  role="option"
                  aria-selected={zone === value}
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-12 hover:bg-accent focus-visible:bg-accent"
                  onClick={() => {
                    onChange(zone);
                    setOpen(false);
                    setSearch("");
                  }}
                >
                  <Check
                    className={`size-3.5 ${zone === value ? "opacity-100" : "opacity-0"}`}
                  />
                  {zone}
                  {zone === suggested && (
                    <span className="text-muted-foreground">(browser)</span>
                  )}
                </button>
              ))}
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}
