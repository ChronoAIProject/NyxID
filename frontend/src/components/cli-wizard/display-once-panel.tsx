// Reusable DisplayOnce panel: renders a one-time secret with a
// copy-to-clipboard affordance, masks the secret until the user clicks
// "Reveal", and owns the "I saved it" → complete-pairing button.
//
// Rendered by every wizard flow that produces a one-time secret
// (api-key create/rotate, node register-token/rotate-token). Consumed
// by both the remote-pairing page (`/cli/pair`) and the CLI's locally-
// served wizard (Mode A) via `wizard-entry.tsx`.
//
// Only the label text and the shape of the ack-payload callback differ
// per kind; that variance lives at the call site.

import { useState } from "react"
import { Button } from "@/components/ui/button"
import { Check, Copy, Eye, EyeOff, Lock } from "lucide-react"

interface DisplayOnceProps {
  readonly title: string
  readonly description: string
  /** The one-time secret to display. Shown masked until Reveal. */
  readonly secret: string
  /** Optional secondary secret (e.g. node rotate has two). */
  readonly secondarySecret?: {
    readonly label: string
    readonly value: string
  }
  /** Button label for the final acknowledgment. */
  readonly ackButtonLabel: string
  /** Called when the user clicks the ack button; pairing-complete POST fires here. */
  readonly onAcknowledge: () => void
  /** True while the /complete POST is in-flight; disables the button. */
  readonly isAcknowledging: boolean
}

export function DisplayOncePanel({
  title,
  description,
  secret,
  secondarySecret,
  ackButtonLabel,
  onAcknowledge,
  isAcknowledging,
}: DisplayOnceProps) {
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2 text-amber-600 dark:text-amber-500">
          <Lock className="h-4 w-4" />
          <span className="text-sm font-medium">Shown once — save it now</span>
        </div>
        <h2 className="font-serif text-[28px] font-normal">{title}</h2>
        <p className="text-sm text-muted-foreground">{description}</p>
      </div>

      <SecretField label="Secret" value={secret} />
      {secondarySecret ? (
        <SecretField label={secondarySecret.label} value={secondarySecret.value} />
      ) : null}

      <div className="flex flex-col gap-2">
        <Button onClick={onAcknowledge} disabled={isAcknowledging} className="w-full">
          {isAcknowledging ? "Notifying CLI..." : ackButtonLabel}
        </Button>
        <p className="text-xs text-muted-foreground">
          This value will not be shown again. Confirm you have saved it before closing this
          tab.
        </p>
      </div>
    </div>
  )
}

function SecretField({ label, value }: { readonly label: string; readonly value: string }) {
  const [revealed, setRevealed] = useState(false)
  const [copied, setCopied] = useState(false)

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(value)
      setCopied(true)
      window.setTimeout(() => {
        setCopied(false)
      }, 2000)
    } catch {
      // Clipboard API may be blocked in non-secure contexts.
      // Fall through; user can still manually select the revealed text.
    }
  }

  const masked = "•".repeat(Math.max(value.length, 12))

  return (
    <div className="flex flex-col gap-2">
      <label className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </label>
      <div className="flex items-center gap-2">
        <code className="flex-1 overflow-x-auto rounded-md border bg-muted/40 px-3 py-2 font-mono text-sm">
          {revealed ? value : masked}
        </code>
        <Button
          type="button"
          variant="outline"
          size="icon"
          onClick={() => {
            setRevealed((r) => !r)
          }}
          aria-label={revealed ? "Hide" : "Reveal"}
        >
          {revealed ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="icon"
          onClick={() => void handleCopy()}
          aria-label="Copy to clipboard"
        >
          {copied ? <Check className="h-4 w-4 text-green-600" /> : <Copy className="h-4 w-4" />}
        </Button>
      </div>
    </div>
  )
}
