import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { NextStepConnect } from "./next-step-connect";
import { ArrowRight } from "lucide-react";
import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import { OnboardingIcon } from "./onboarding-icons";
import { cn } from "@/lib/utils";
import type { NextStep } from "@/schemas/assistant-onboarding";

/**
 * Connection suggestions open hosted setup first; other next steps send
 * their prompt to NyxBot with its normal tools and confirmations.
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
  useUiTranslation();

  const [connecting, setConnecting] = useState<NextStep>();
  // OAuth completion can remove this recommendation during a Home refresh.
  // Retain the selected row until its hosted setup delivers the follow-up.
  const displayed = connecting && !steps.some((step) => step.id === connecting.id)
    ? [connecting, ...steps] : steps;
  return (
    <ul
      aria-label={translateUi("Suggested next steps")}
      className={cn("grid gap-2", className)}
    >
      {displayed.map((step, index) => {
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
                  onClick={() => step.connect ? setConnecting(step) : onStart(step)}
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
                    <span className="block break-words type-row-title text-foreground">
                      {step.title}
                    </span>
                    <span className="block type-secondary text-muted-foreground">
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
            {connecting?.id === step.id ? <NextStepConnect key={step.id} step={step} onStart={onStart} onClose={() => setConnecting(undefined)} /> : null}
          </li>
        );
      })}
    </ul>
  );
}
