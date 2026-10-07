import { useState, type ReactNode } from "react";
import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import {
  Popover,
  PopoverTrigger,
  PopoverContent,
} from "@/components/ui/popover";

export interface ServiceFilterOption {
  readonly id: string;
  readonly label: string;
  readonly icon?: ReactNode;
}

export function ServiceFilterMultiselect({
  label,
  plural,
  options,
  selected,
  onChange,
  className,
}: {
  readonly label: string;
  readonly plural: string;
  readonly options: readonly ServiceFilterOption[];
  readonly selected: readonly string[];
  readonly onChange: (ids: string[]) => void;
  readonly className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const hasIcons = options.some((option) => option.icon);
  const visible = options.filter((option) =>
    option.label.toLowerCase().includes(search.trim().toLowerCase()),
  );
  const summary =
    selected.length === 0
      ? "All"
      : selected.length === 1
        ? (options.find((option) => option.id === selected[0])?.label ??
          "Unavailable selection")
        : `${selected.length} selected`;
  return (
    <Popover
      open={open}
      onOpenChange={(value) => {
        setOpen(value);
        if (!value) setSearch("");
      }}
    >
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          aria-label={label}
          className={cn(
            "grid h-11 w-full grid-cols-[6rem_minmax(0,1fr)_auto] items-center gap-2 text-left font-normal sm:w-60 md:h-9",
            className,
          )}
        >
          <span className="shrink-0 text-muted-foreground">{label}</span>
          <span className="min-w-0 truncate text-right" title={summary}>
            {summary}
          </span>
          <ChevronDown
            className="size-3.5 shrink-0 text-muted-foreground"
            aria-hidden="true"
          />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        aria-label={`${label} filter`}
        className="w-72 max-w-[calc(100vw-2rem)] space-y-2 p-2"
      >
        <Input
          aria-label={`Search ${plural}`}
          placeholder={`Search ${plural}…`}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        <div
          className="max-h-64 overflow-y-auto overscroll-contain"
          role="group"
          aria-label={`Select ${plural}`}
        >
          {visible.map((option) => {
            const checked = selected.includes(option.id);
            return (
              <label
                key={option.id}
                className={cn(
                  "grid cursor-pointer items-center gap-2 rounded-md px-2 py-2 text-left text-xs hover:bg-accent focus-within:bg-accent",
                  hasIcons
                    ? "grid-cols-[1rem_1.25rem_minmax(0,1fr)]"
                    : "grid-cols-[1rem_minmax(0,1fr)]",
                )}
              >
                <Checkbox
                  aria-label={option.label}
                  checked={checked}
                  disabled={!checked && selected.length >= 100}
                  onCheckedChange={(value) =>
                    onChange(
                      value === true
                        ? [...selected, option.id]
                        : selected.filter((id) => id !== option.id),
                    )
                  }
                />
                {hasIcons && (
                  <span className="flex size-5 items-center justify-center">
                    {option.icon}
                  </span>
                )}
                <span className="min-w-0 break-words leading-5">
                  {option.label}
                </span>
              </label>
            );
          })}
          {!visible.length && (
            <p className="px-2 py-4 text-xs text-muted-foreground">
              No matching {plural}.
            </p>
          )}
        </div>
        <div className="flex items-center justify-between border-t border-border pt-2">
          <Button
            size="sm"
            variant="ghost"
            aria-label={`Clear ${plural}`}
            disabled={!selected.length}
            onClick={() => onChange([])}
          >
            Clear
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setOpen(false);
              setSearch("");
            }}
          >
            Done
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
