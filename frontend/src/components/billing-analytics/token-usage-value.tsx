import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useState } from "react";
import type { AdminUsageStats } from "@/types/admin";
import { MEASURE_LABELS, formatAnalyticsValue } from "@/lib/usage-analytics";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { sumTokenMetrics, type TokenMetric } from "./token-metrics";

export function TokenUsageValue({
  usage,
  selected,
}: {
  usage: AdminUsageStats;
  selected: readonly TokenMetric[];
}) {
  useUiTranslation();
  const [open, setOpen] = useState(false);
  const totalSelected = selected.includes("total_tokens");
  const value = sumTokenMetrics(usage, selected);
  const parts = totalSelected
    ? (["prompt_tokens", "completion_tokens"] as const)
    : selected;
  const number = (value: number) => value.toLocaleString(getFormattingLocale());
  const details = [
    { label: "Image input", value: usage.image_input_tokens },
    { label: "Image output", value: usage.image_output_tokens },
    { label: "Voice input", value: usage.audio_input_tokens },
    { label: "Voice output", value: usage.audio_output_tokens },
  ].filter((detail) => detail.value > 0);

  return (
    <Tooltip open={open} onOpenChange={setOpen}>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={translateUi("Token count breakdown: {{value1}} tokens", { value1: number(value) })}
          onClick={(event) => {
            // Radix closes tooltips on click; keep the breakdown visible after a tap.
            event.preventDefault();
            setOpen(true);
          }}
          className="cursor-pointer rounded-sm text-left font-display type-metric tracking-tight tabular-nums outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-card"
        >
          {formatAnalyticsValue(value, "tokens", true)}
        </button>
      </TooltipTrigger>
      <TooltipContent
        collisionPadding={12}
        className="w-80 max-w-[calc(100vw-1.5rem)] space-y-3 border border-border bg-card p-3 text-13 text-foreground"
      >
        <div>
          <p className="font-medium">
            {totalSelected ? translateUi("Total tokens") : translateUi("Selected token counts")}
          </p>
          <p className="mt-1 font-mono text-13 tabular-nums">
            {parts.map((metric) => number(usage[metric])).join(" + ")} ={" "}
            {number(value)}
          </p>
        </div>
        <dl className="space-y-1.5">
          {parts.map((metric) => (
            <div key={metric} className="flex justify-between gap-6">
              <dt className="text-muted-foreground">
                {MEASURE_LABELS[metric]}
              </dt>
              <dd className="tabular-nums">{number(usage[metric])}</dd>
            </div>
          ))}
        </dl>
        {totalSelected && (
          <div className="space-y-1.5 border-t border-border pt-2">
            {details.length > 0 && (
              <>
                <p className="font-medium">{translateUi("Included in input / output")}</p>
                <dl className="space-y-1.5">
                  {details.map((detail) => (
                    <div
                      key={detail.label}
                      className="flex justify-between gap-6"
                    >
                      <dt className="text-muted-foreground">{detail.label}</dt>
                      <dd className="tabular-nums">{number(detail.value)}</dd>
                    </div>
                  ))}
                </dl>
              </>
            )}
            <p className="text-13 text-muted-foreground">{translateUi("Image and voice tokens are included in the total when reported.")}{details.length === 0 &&
                " No separate image or voice breakdown was recorded for this selection."}
            </p>
            {(usage.cached_tokens > 0 || usage.cache_creation_tokens > 0) && (
              <p className="text-13 text-muted-foreground">{translateUi("Cache read ")}{number(usage.cached_tokens)}{translateUi(" · cache write")}{" "}
                {number(usage.cache_creation_tokens)}. These can overlap input
                counts and are not added to the total.
              </p>
            )}
          </div>
        )}
        {!totalSelected && selected.length > 1 && (
          <p className="text-13 text-muted-foreground">{translateUi("Selected counts are added together. Cache counts can overlap input; use Total tokens for the input + output total.")}</p>
        )}
      </TooltipContent>
    </Tooltip>
  );
}
