/**
 * Shared wizard shell — the header/main/footer chrome rendered around
 * every step of every wizard flow, in both Mode A (local wizard served
 * by the CLI's embedded axum server) and Mode B (remote pairing via
 * `/cli/pair` on the frontend).
 *
 * Structure mirrors Mode A's `.wizard-shell > .wizard-header + .wizard-main
 * + .wizard-footer` layout from `cli/src/wizard/assets/wizard.html:13-19,
 * 22, 358-361` and `cli/src/wizard/assets/wizard.css:57-104, 754-769`.
 * Typography (DM Serif Display wordmark at 24px, void-300 colour) matches
 * `.wizard-brand-wordmark` at `wizard.css:85-92`.
 *
 * Design-token mapping (Mode A → frontend Tailwind @theme token):
 *   --panel       → bg-card
 *   --border      → border-border
 *   --muted       → text-muted-foreground
 *   --wordmark    → text-nyx-200
 *   --primary     → text-primary
 */

import type { ReactNode } from "react"
import { WizardFooter } from "./wizard-footer"
import { formatStepLabel, type WizardStep } from "./step-label"

export interface WizardShellProps {
  readonly step?: WizardStep
  readonly context: "local" | "pair"
  readonly localOrigin?: string
  readonly children: ReactNode
}

export function WizardShell({ step, context, localOrigin, children }: WizardShellProps) {
  return (
    <div className="min-h-screen bg-background text-foreground">
    <div className="mx-auto flex min-h-screen max-h-screen w-full max-w-[1040px] flex-col px-6 pt-10 pb-6">
      <header className="mb-6 flex items-center justify-between">
        <div className="flex items-center">
          <img src="/nyxid-wordmark.svg" alt="NyxID" className="h-9 w-auto" />
        </div>
        {step ? (
          <div className="text-[12px] text-muted-foreground">{formatStepLabel(step)}</div>
        ) : null}
      </header>
      <main className="flex-1 min-h-0 overflow-y-auto overscroll-contain rounded-xl border border-border bg-card p-8">
        {children}
      </main>
      <WizardFooter context={context} localOrigin={localOrigin} />
    </div>
    </div>
  )
}
