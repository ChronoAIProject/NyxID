import { ArrowRight } from "lucide-react";
import { OnboardingIcon } from "./onboarding-icons";
import { cn } from "@/lib/utils";
import type { NextStep } from "@/schemas/assistant-onboarding";


/**
 * One-click next steps. Starting one sends its prompt to NyxBot, which does
 * the setup with its normal tools and confirmations.
 */
export function NextStepCards({
  steps,
  onStart,
  disabled,
  className,
}: {
  readonly steps: readonly NextStep[];
  readonly onStart: (step: NextStep) => void;
  readonly disabled?: boolean;
  readonly className?: string;
}) {
  return (
    <ul aria-label="Suggested next steps" className={cn("grid gap-2", className)}>
      {steps.map((step, index) => {
        return (
          <li
            key={step.id}
            className="group relative animate-in fade-in-0 slide-in-from-bottom-1 duration-300 motion-reduce:animate-none"
            style={{ animationDelay: `${index * 60}ms`, animationFillMode: "both" }}
          >
            <button
              type="button"
              disabled={disabled}
              onClick={() => onStart(step)}
              className={cn(
                "flex w-full items-center gap-3 rounded-lg border border-border bg-card px-3 py-2.5 text-left shadow-sm transition-colors",
                "hover:border-hairline-strong hover:bg-overlay focus-visible:border-input-focus focus-visible:outline-none",
                "disabled:cursor-not-allowed disabled:opacity-60 motion-reduce:transition-none",
              )}
            >
              <span
                aria-hidden="true"
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay text-foreground"
              >
                <OnboardingIcon name={step.icon} className="size-4" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-13 font-medium text-foreground">
                  {step.title}
                </span>
                <span className="block text-11 leading-snug text-muted-foreground">
                  {step.description}
                </span>
              </span>
              <ArrowRight
                aria-hidden="true"
                className="size-3.5 shrink-0 text-text-tertiary transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none"
              />
            </button>
          </li>
        );
      })}
    </ul>
  );
}
