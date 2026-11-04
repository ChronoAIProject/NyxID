import { ArrowRight } from "lucide-react";
import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
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
    <ul
      aria-label="Suggested next steps"
      className={cn("grid gap-2", className)}
    >
      {steps.map((step, index) => {
        return (
          <li
            key={step.id}
            className="group relative animate-in fade-in-0 slide-in-from-bottom-1 duration-300 motion-reduce:animate-none"
            style={{
              animationDelay: `${index * 60}ms`,
              animationFillMode: "both",
            }}
          >
            <GroupedList>
              <GroupedRow asChild interactive className="group">
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() => onStart(step)}
                  className={cn(
                    "flex w-full items-center gap-3 text-left transition-colors",
                    "disabled:cursor-not-allowed disabled:opacity-60 motion-reduce:transition-none",
                  )}
                >
                  <span
                    aria-hidden="true"
                    className="shrink-0 text-text-tertiary"
                  >
                    <OnboardingIcon
                      name={step.icon}
                      className="size-[1.125em]"
                    />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block break-words text-14 font-medium text-foreground">
                      {step.title}
                    </span>
                    <span className="block text-13 font-normal leading-snug text-muted-foreground">
                      {step.description}
                    </span>
                  </span>
                  <ArrowRight
                    aria-hidden="true"
                    className="size-[1.125em] shrink-0 text-text-tertiary transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none"
                  />
                </button>
              </GroupedRow>
            </GroupedList>
          </li>
        );
      })}
    </ul>
  );
}
