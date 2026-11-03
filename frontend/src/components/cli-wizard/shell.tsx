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
 *   --primary     → text-primary-text
 */

import { useRef, type ReactNode } from "react"
import { WizardFooter } from "./wizard-footer"
import { formatStepLabel, type WizardStep } from "./step-label"
import { useApplyTheme } from "@/hooks/use-theme"
import { useScrollFades } from "@/hooks/use-scroll-fades"
import { NyxidLogo } from "@/components/brand/nyxid-logo"

export interface WizardShellProps {
  readonly step?: WizardStep
  readonly context: "local" | "pair"
  readonly localOrigin?: string
  /**
   * Single-form steps (a picked service) use the narrow column of the hosted
   * connector and OAuth consent pages; the catalog grid keeps the wide one.
   */
  readonly narrow?: boolean
  readonly children: ReactNode
}

export function WizardShell({
  step,
  context,
  localOrigin,
  narrow = false,
  children,
}: WizardShellProps) {
  useApplyTheme()
  const mainRef = useRef<HTMLElement>(null)
  useScrollFades(mainRef)
  return (
    <div className="min-h-screen bg-background text-foreground">
    {/*
      Header + footer chrome stays within 20% of the window: 182px from
      920px up, 134px in the `short` tier (680-919px), and 72px below that,
      where the footer is hidden entirely (see WizardFooter).
    */}
    <div
      className={`mx-auto flex max-h-screen w-full ${narrow ? "max-w-[640px]" : "max-w-[1040px]"} flex-col px-6 pt-10 pb-6 short:pt-6 short:pb-4 very-short:pt-4 very-short:pb-4`}
    >
      <header className="mb-6 flex items-center justify-between short:mb-4 very-short:mb-4">
        <div className="flex items-center">
          <NyxidLogo className="h-9 w-auto short:h-7 very-short:h-6" />
        </div>
        {step ? (
          <div className="text-12 text-muted-foreground">{formatStepLabel(step)}</div>
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

        The card chrome stays on main and the scrolling moves to the inner
        region, so the edge fades dim the content but never the border.
        `relative` makes the region the containing block for absolutely
        positioned descendants (`sr-only` labels): otherwise they resolve
        against the page and stretch it into a second scrollbar.
      */}
      <main
        ref={mainRef}
        className="flex min-h-[240px] flex-col overflow-hidden rounded-xl border border-border bg-card"
      >
        <div
          data-scroll-fade=""
          className="relative min-h-0 flex-1 overflow-y-auto overscroll-contain p-8"
        >
          {children}
        </div>
      </main>
      <WizardFooter context={context} localOrigin={localOrigin} />
    </div>
    </div>
  )
}
