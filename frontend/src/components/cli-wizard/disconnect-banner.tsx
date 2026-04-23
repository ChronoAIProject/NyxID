/**
 * Dismissible-at-your-own-risk banner shown at the top of the wizard
 * when the browser has lost contact with the CLI (Mode A) or the
 * pairing record has gone stale (Mode B).
 *
 * Non-blocking — the user can still copy a displayed secret or take
 * screenshots. But the action buttons get disabled upstream so a
 * dead session can't process clicks that will never reach anyone.
 */

import { WifiOff, Loader2 } from "lucide-react"

export interface DisconnectBannerProps {
  /** `disconnected` = connection lost, `reconnecting` = trying again. */
  readonly state: "disconnected" | "reconnecting"
  /** Source of the disconnect — drives the copy. */
  readonly context: "local" | "pair"
  /** Optional: status of the pairing record when context is "pair". */
  readonly pairingStatus?: "cancelled" | "expired" | "unknown"
}

export function DisconnectBanner({
  state,
  context,
  pairingStatus,
}: DisconnectBannerProps) {
  const Icon = state === "reconnecting" ? Loader2 : WifiOff
  const title =
    state === "reconnecting"
      ? "Reconnecting…"
      : context === "local"
        ? "Connection to CLI lost"
        : pairingStatus === "cancelled"
          ? "CLI cancelled this pairing"
          : pairingStatus === "expired"
            ? "Pairing expired"
            : "Pairing went stale"

  const body =
    state === "reconnecting"
      ? "Retrying the last check…"
      : context === "local"
        ? "The nyxid CLI stopped responding. It may have exited, been suspended, or hit a network issue. You can keep this tab open to copy anything displayed, but the flow cannot complete from here — re-run the command in your terminal."
        : pairingStatus === "cancelled"
          ? "The CLI sent a cancel — nothing was created on the server. You can close this tab."
          : pairingStatus === "expired"
            ? "This pairing passed its 15-minute TTL. Re-run the command in your terminal to start a new one."
            : "The pairing record is no longer reachable. Re-run the CLI command to start a fresh one."

  return (
    <div
      role="alert"
      aria-live="polite"
      className="mb-4 flex items-start gap-3 rounded-[10px] border border-destructive/50 bg-destructive/10 px-4 py-3 text-[13px] text-foreground"
    >
      <Icon
        className={
          "mt-0.5 h-4 w-4 shrink-0 text-destructive " +
          (state === "reconnecting" ? "animate-spin" : "")
        }
        aria-hidden
      />
      <div className="flex flex-col gap-1">
        <p className="font-medium">{title}</p>
        <p className="text-muted-foreground">{body}</p>
      </div>
    </div>
  )
}
