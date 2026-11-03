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
    <div className="flex flex-col items-center gap-2 text-center">
      {eyebrow}
      <h2 className="text-xl font-semibold leading-tight text-foreground sm:text-2xl">
        {title}
      </h2>
      {children ? (
        <div className="max-w-md text-sm text-muted-foreground">{children}</div>
      ) : null}
    </div>
  )
}
