import { Label } from "@/components/ui/label";
import { GroupedRow } from "@/components/ui/grouped-list";
import { useRef, useState } from "react";
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
  grouped = false,
  describedBy,
  invalid,
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly label?: string;
  readonly grouped?: boolean;
  readonly describedBy?: string;
  readonly invalid?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const resetOnOpen = useRef(false);
  const suggested = browserTimezone();
  const zones = [
    ...new Set([
      suggested,
      "UTC",
      value,
      ...Intl.supportedValuesOf("timeZone"),
    ]),
  ].filter(Boolean);
  const picker = (
    <Popover open={open} onOpenChange={(next) => {
      if (next && resetOnOpen.current) {
        setSearch("");
        resetOnOpen.current = false;
      }
      setOpen(next);
    }}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant={grouped ? "ghost" : "outline"}
          role="combobox"
          aria-label={label}
          aria-expanded={open}
          aria-describedby={describedBy}
          aria-invalid={invalid}
          className={
            grouped
              ? "h-auto min-h-8 max-w-full gap-2 whitespace-normal px-0 text-right"
              : "w-full justify-between"
          }
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
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left type-menu-item hover:bg-accent focus-visible:bg-accent"
                onClick={() => {
                  onChange(zone);
                  // Keep the closing surface stable while Floating UI measures it.
                  // Clear the selection search before the next surface mounts.
                  resetOnOpen.current = true;
                  setOpen(false);
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
  );
  return grouped ? (
    <GroupedRow title={label} accessory={picker} />
  ) : (
    <div className="space-y-1 text-14">
      <Label asChild><span>{label}</span></Label>
      {picker}
    </div>
  );
}
