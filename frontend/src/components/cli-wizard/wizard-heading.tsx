import type { ReactNode } from "react"

/**
 * Page heading for every wizard step, matching the hosted connector and OAuth
 * consent pages: a centred semibold title over a muted subtitle.
 */
export function WizardHeading({
  title,
  eyebrow,
  children,
}: {
  readonly title: ReactNode
  /** Optional badge above the title, e.g. the display-once warning. */
  readonly eyebrow?: ReactNode
  /** Subtitle copy. */
  readonly children?: ReactNode
}) {
  return (
    <div className="flex flex-col gap-2 border-b border-border/60 pb-6">
      {eyebrow}
      <h2 className="type-page-title text-foreground">
        {title}
      </h2>
      {children ? (
        <div className="text-14 text-muted-foreground">{children}</div>
      ) : null}
    </div>
  )
}
