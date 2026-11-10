import type { ReactNode } from "react";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { useApplyTheme } from "@/hooks/use-theme";
import { useWizardTranslation } from "@/i18n/wizard";
import { cn } from "@/lib/utils";
import { WizardLanguagePicker } from "./language-picker";
import { formatStepLabel, type WizardStep } from "./step-label";
import { WizardFooter } from "./wizard-footer";
import "./wizard.css";

export interface WizardShellProps {
  readonly step?: WizardStep;
  readonly context: "local" | "pair";
  readonly localOrigin?: string;
  readonly children: ReactNode;
}

export function WizardShell({
  step,
  context,
  localOrigin,
  children,
}: WizardShellProps) {
  const { t } = useWizardTranslation();
  useApplyTheme();
  return (
    <div className="nyxid-wizard min-h-screen bg-background text-foreground [--radius-lg:8px] [--radius-md:6px]">
      <div className="mx-auto flex max-h-screen w-full max-w-[800px] flex-col px-4 pt-6 pb-5 sm:px-6 sm:pt-10 sm:pb-6">
        <header className="mb-5 shrink-0">
          <div className="flex items-center justify-between gap-3">
            <NyxidLogo className="h-7 w-auto" />
            <WizardLanguagePicker />
          </div>
          {step ? (
            <div className="mt-6 flex items-center justify-between gap-4">
              <p className="text-12 leading-relaxed text-muted-foreground">
                {formatStepLabel(step, t)}
              </p>
              {step.label.startsWith("Recovery") ? null : (
                <div aria-hidden="true" className="flex shrink-0 gap-1.5">
                  {Array.from({ length: step.total }, (_, index) => (
                    <span
                      key={index}
                      className={cn(
                        "h-1 w-5 rounded-full sm:w-7",
                        index < step.current ? "bg-primary" : "bg-border/70",
                      )}
                    />
                  ))}
                </div>
              )}
            </div>
          ) : null}
        </header>
        {/* Keep short steps compact and long steps scrollable within the card. */}
        <main className="min-h-[240px] overflow-y-auto overscroll-contain rounded-xl border border-border/70 bg-card p-5 shadow-sm sm:p-8">
          {children}
        </main>
        <WizardFooter context={context} localOrigin={localOrigin} />
      </div>
    </div>
  );
}
