import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { cn } from "@/lib/utils";

/**
 * Step indicator shared by guided flows: a wide bar for the current step and
 * short dots for the rest. Steps the caller marks reachable are buttons.
 */
export function StepDots({
  label,
  steps,
  index,
  onSelect,
  canSelect = () => true,
  showCaption = false,
  className,
}: {
  /** Accessible name of the whole indicator. */
  readonly label: string;
  /** One human-readable label per step, in order. */
  readonly steps: readonly string[];
  readonly index: number;
  readonly onSelect: (position: number) => void;
  readonly canSelect?: (position: number) => boolean;
  /** Prints "Step 2 of 4 · Actions" beside the dots. */
  readonly showCaption?: boolean;
  readonly className?: string;
}) {
  useUiTranslation();
  return (
    <nav aria-label={label} className={cn("flex items-center px-0.75", className)}>
      {steps.map((step, position) => (
        <button
          key={step}
          type="button"
          title={step}
          aria-label={translateUi("Step {{value1}} of {{value2}}: {{value3}}", {
            value1: position + 1,
            value2: steps.length,
            value3: step,
          })}
          aria-current={position === index ? "step" : undefined}
          disabled={!canSelect(position)}
          onClick={() => onSelect(position)}
          data-icon-button=""
          className="group -mx-0.75 flex items-center justify-center rounded-full px-0.75 py-2 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-input-focus disabled:cursor-default"
        >
          <span
            aria-hidden="true"
            className={cn(
              "block h-1 rounded-full transition-all duration-300 motion-reduce:transition-none",
              position === index
                ? "w-6 bg-foreground"
                : position < index
                  ? "w-3 bg-muted-foreground group-enabled:group-hover:w-4 group-enabled:group-hover:bg-foreground"
                  : "w-3 bg-overlay-strong group-enabled:group-hover:w-4 group-enabled:group-hover:bg-muted-foreground",
            )}
          />
        </button>
      ))}
      {showCaption ? (
        <span aria-hidden="true" className="ml-3 type-secondary text-text-tertiary">
          {translateUi("Step {{value1}} of {{value2}}", { value1: index + 1, value2: steps.length })}
          {" · "}
          {steps[index]}
        </span>
      ) : null}
    </nav>
  );
}
