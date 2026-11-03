import { ChevronLeft } from "lucide-react"

/**
 * The wizard's single Back control: a round chevron button pinned to the
 * card's top-left corner. The zero-height sticky anchor keeps it in place
 * while the step scrolls without taking up layout space.
 */
export function WizardBackFab({ onBack }: { readonly onBack: () => void }) {
  return (
    // Sticky offsets are measured from the card's content edge (inside its
    // padding), so `top-0` keeps the button in the same corner spot before
    // and after the step scrolls. Sitting 20px in keeps it clear of the
    // card's 20px top scroll fade, which would otherwise dim its edge.
    <div className="sticky top-0 z-20 h-0">
      {/*
        Opaque `muted` fill with a strong hairline rim, one step off the card
        colour in either theme, so the button stands apart from content
        scrolling beneath it. No `title`: `aria-label` already names it, and
        the native tooltip only repeated "Back".
      */}
      <button
        type="button"
        onClick={onBack}
        aria-label="Back"
        className="absolute -left-3 -top-3 flex size-9 items-center justify-center rounded-full border border-hairline-strong bg-muted text-foreground shadow-sm transition-[filter] hover:brightness-125 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      >
        <ChevronLeft className="size-4" aria-hidden="true" />
      </button>
    </div>
  )
}
