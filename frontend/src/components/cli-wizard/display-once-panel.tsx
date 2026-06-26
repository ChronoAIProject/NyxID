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
import { Label } from "@/components/ui/label"
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
          <span className="text-[12px] font-medium">Shown once — save it now</span>
        </div>
        <h2 className="font-serif text-[28px] font-normal">{title}</h2>
        <p className="text-[12px] text-muted-foreground">{description}</p>
      </div>

      <SecretField label="Secret" value={secret} />
      {secondarySecret ? (
        <SecretField label={secondarySecret.label} value={secondarySecret.value} />
      ) : null}

      <div className="flex flex-col gap-2">
        <Button variant="primary" onClick={onAcknowledge} disabled={isAcknowledging} className="w-full">
          {isAcknowledging ? "Notifying CLI..." : ackButtonLabel}
        </Button>
        <p className="text-xs text-muted-foreground">
          This value will not be shown again. Confirm you have saved it before closing this
          tab.
        </p>
        <p className="text-xs text-muted-foreground">
          <strong>Lost it?</strong> If you don&apos;t copy this now,
          you&apos;ll need to rotate the credential to get a new one. (Old
          value stops working immediately on rotate.)
        </p>
      </div>
    </div>
  )
}

/**
 * Display-once panel for MFA recovery codes (issue #506). Renders the
 * code list masked-by-default and offers reveal / copy-all / download
 * affordances. Same single-ack contract as `DisplayOncePanel` — the
 * caller fires `/cli-pairings/{id}/complete` (Mode B) or
 * `/api/proxy/complete` (Mode A) when the user clicks the ack button.
 */
export function RecoveryCodesPanel({
  codes,
  onAcknowledged,
}: {
  readonly codes: readonly string[]
  readonly onAcknowledged: () => void
}) {
  const [revealed, setRevealed] = useState(false)
  const [copied, setCopied] = useState(false)

  async function copyAll() {
    try {
      await navigator.clipboard.writeText(codes.join("\n"))
      setCopied(true)
      window.setTimeout(() => {
        setCopied(false)
      }, 2000)
    } catch {
      // Clipboard API may be blocked; user can manually select revealed text.
    }
  }

  function downloadTxt() {
    const header =
      "NyxID MFA recovery codes — store these securely.\n" +
      "Each code can be used ONCE if you lose access to your authenticator.\n\n"
    const blob = new Blob([header + codes.join("\n") + "\n"], {
      type: "text/plain",
    })
    const url = URL.createObjectURL(blob)
    const link = document.createElement("a")
    link.href = url
    link.download = "nyxid-mfa-recovery-codes.txt"
    document.body.appendChild(link)
    link.click()
    document.body.removeChild(link)
    URL.revokeObjectURL(url)
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2 text-amber-600 dark:text-amber-500">
          <Lock className="h-4 w-4" />
          <span className="text-sm font-medium">Shown once — save them now</span>
        </div>
        <h2 className="font-serif text-[28px] font-normal">Save your recovery codes</h2>
        <p className="text-sm text-muted-foreground">
          Each code is single-use and lets you sign in if you lose access to your
          authenticator. Store them in a password manager or print them. They are{" "}
          <strong>not</strong> shown again.
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <Label>Recovery codes</Label>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              setRevealed((r) => !r)
            }}
          >
            {revealed ? "Hide" : "Reveal"}
          </Button>
        </div>
        <div className="grid grid-cols-1 gap-1.5 rounded-md border bg-muted/40 p-3 font-mono text-sm sm:grid-cols-2">
          {codes.map((c, i) => (
            <code key={i} className="select-text">
              {revealed ? c : "•".repeat(Math.max(c.length, 12))}
            </code>
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-2 sm:flex-row">
        <Button
          type="button"
          variant="outline"
          onClick={() => void copyAll()}
          className="flex-1"
        >
          {copied ? "Copied!" : "Copy all"}
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={downloadTxt}
          className="flex-1"
        >
          Download .txt
        </Button>
      </div>

      <Button onClick={onAcknowledged}>I have saved them — close</Button>
      <p className="text-xs text-muted-foreground">
        After closing, these codes cannot be retrieved. Re-run{" "}
        <code className="font-mono">nyxid mfa setup</code> to mint a new set if you didn't
        save them.
      </p>
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
        <code className="flex-1 overflow-x-auto rounded-lg border bg-muted/40 px-3 py-2 font-mono text-[12px]">
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
