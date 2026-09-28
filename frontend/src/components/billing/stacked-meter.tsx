import { useState } from "react";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  overallCaption,
  stackStatusClass,
  stackValueText,
  type Stack,
} from "@/lib/benefit-stack";
import { cn } from "@/lib/utils";

/** The legend/row marker for one segment, at that segment's opacity step. */
export function StackSwatch({ step }: { step: number }) {
  return (
    <span className={cn("stack-swatch", `stack-step-${step}`)} aria-hidden />
  );
}

/**
 * One stacked bar: contiguous segments from the left, one per entry, each a
 * step lighter than the last. Hovering a segment names it; focusing the bar
 * lists every entry, and screen readers get the same breakdown.
 */
export function StackedMeter({
  stack,
  label,
}: {
  stack: Stack;
  label: string;
}) {
  const [focused, setFocused] = useState(false);
  const segments = stack.entries.filter((entry) => entry.share > 0);
  return (
    <span className="benefit-meter stacked-meter">
      <Tooltip open={focused} onOpenChange={() => undefined}>
        <TooltipTrigger asChild>
          <span
            className="stack-track"
            role="meter"
            tabIndex={0}
            aria-label={`${label} used`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={stack.overall}
            aria-valuetext={stackValueText(stack)}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
          >
            {segments.map((entry) => (
              <Tooltip key={entry.key} delayDuration={100}>
                <TooltipTrigger asChild>
                  <span
                    className={cn("stack-seg", `stack-step-${entry.step}`)}
                    data-key={entry.key}
                    style={{ width: `${round(entry.share)}%` }}
                  />
                </TooltipTrigger>
                <TooltipContent className="whitespace-pre-line">
                  {entry.tooltip}
                </TooltipContent>
              </Tooltip>
            ))}
          </span>
        </TooltipTrigger>
        <TooltipContent
          className={cn(
            "billing-stack-tooltip",
            stackStatusClass(stack.status),
          )}
          side="top"
        >
          <p>{overallCaption(stack.overall)}</p>
          <ul>
            {stack.items.map((item) => (
              <li key={item.key}>
                <StackSwatch step={item.step} />
                {item.label} · {item.detail}
              </li>
            ))}
          </ul>
        </TooltipContent>
      </Tooltip>
      <span className="benefit-meter-caption">
        {overallCaption(stack.overall)}
      </span>
    </span>
  );
}

function round(value: number) {
  return Math.round(value * 10_000) / 10_000;
}
