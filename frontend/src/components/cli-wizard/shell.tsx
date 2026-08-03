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
import { useApplyTheme } from "@/hooks/use-theme"
import { NyxidLogo } from "@/components/brand/nyxid-logo"

export interface WizardShellProps {
  readonly step?: WizardStep
  readonly context: "local" | "pair"
  readonly localOrigin?: string
  readonly children: ReactNode
}

export function WizardShell({ step, context, localOrigin, children }: WizardShellProps) {
  useApplyTheme()
  return (
    <div className="min-h-screen bg-background text-foreground">
    <div className="mx-auto flex max-h-screen w-full max-w-[1040px] flex-col px-6 pt-10 pb-6">
      <header className="mb-6 flex items-center justify-between">
        <div className="flex items-center">
          <NyxidLogo className="h-9 w-auto" />
        </div>
        {step ? (
          <div className="text-[12px] text-muted-foreground">{formatStepLabel(step)}</div>
        ) : null}
      </header>
      {/*
        No `flex-1` — main hugs its content, so a short step ends the card
        at the content instead of stretching it to the viewport bottom.
        `min-h-[240px]` floors transient states (a "Loading catalog…" step
        must not render as a sliver that then jumps), and it also serves as
        the flex-shrink override `min-h-0` used to provide — they set the
        same property, so only one can win. 240px still lets main shrink
        far below its content height, so a tall step scrolls internally
        inside the column's `max-h-screen`.
      */}
      <main className="min-h-[240px] overflow-y-auto overscroll-contain rounded-xl border border-border bg-card p-8">
        {children}
      </main>
      <WizardFooter context={context} localOrigin={localOrigin} />
    </div>
    </div>
  )
}
