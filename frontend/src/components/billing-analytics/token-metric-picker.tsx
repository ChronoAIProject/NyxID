import { ChevronDown, Info } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { MEASURE_LABELS } from "@/lib/usage-analytics";
import { cn } from "@/lib/utils";
import {
  TOKEN_METRICS,
  hasRedundantTokenSelection,
  type TokenMetric,
} from "./token-metrics";

export function TokenOverlapHint({
  selected,
}: {
  selected: readonly TokenMetric[];
}) {
  if (!hasRedundantTokenSelection(selected)) return null;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label="How selected token counts are calculated"
          className="inline-flex size-4 shrink-0 items-center justify-center align-middle text-muted-foreground hover:text-foreground"
        >
          <Info className="size-3.5" />
        </button>
      </TooltipTrigger>
      <TooltipContent className="max-w-64">
        Total tokens already includes input and output. Selected input and
        output counts are shown but not added again.
      </TooltipContent>
    </Tooltip>
  );
}

export function TokenMetricCaption({
  selected,
}: {
  selected: readonly TokenMetric[];
}) {
  return (
    <div className="mt-2 flex min-w-0 items-start text-10 leading-relaxed text-muted-foreground">
      <span className="min-w-0 break-words">
        {selected.map((metric) => MEASURE_LABELS[metric]).join(" · ")}
        {hasRedundantTokenSelection(selected) && (
          <>
            {" "}
            <TokenOverlapHint selected={selected} />
          </>
        )}
      </span>
    </div>
  );
}

export function TokenMetricPicker({
  label,
  selected,
  onChange,
  compact = false,
  allLabel,
}: {
  label: string;
  selected: readonly TokenMetric[];
  onChange: (selected: TokenMetric[]) => void;
  compact?: boolean;
  allLabel?: string;
}) {
  const allSelected = selected.length === TOKEN_METRICS.length;
  const custom = selected.length > 1 && !(allSelected && allLabel);
  const name =
    allSelected && allLabel
      ? allLabel
      : selected.length === 1
        ? MEASURE_LABELS[selected[0]!]
        : "Custom";

  function toggle(metric: TokenMetric, checked: boolean) {
    const next = TOKEN_METRICS.filter((field) =>
      field === metric ? checked : selected.includes(field),
    );
    if (next.length > 0) onChange([...next]);
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="sm"
          aria-label={label}
          className={cn(
            "min-w-0 justify-between",
            compact
              ? "h-6 w-full border-0 px-0 text-10 uppercase shadow-none"
              : "w-full sm:w-auto sm:min-w-40",
          )}
        >
          <span className="truncate">{name}</span>
          {custom && (
            <>
              {" "}
              <span className="shrink-0 rounded border border-border px-1 text-10 leading-4">
                ({selected.length})
              </span>
            </>
          )}
          <ChevronDown className="ml-auto size-3 shrink-0" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-52 space-y-0.5 p-2">
        {TOKEN_METRICS.map((metric) => (
          <label
            key={metric}
            className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-12 hover:bg-overlay"
          >
            <Checkbox
              checked={selected.includes(metric)}
              onCheckedChange={(checked) => toggle(metric, checked === true)}
              disabled={selected.length === 1 && selected[0] === metric}
            />
            {MEASURE_LABELS[metric]}
          </label>
        ))}
      </PopoverContent>
    </Popover>
  );
}
