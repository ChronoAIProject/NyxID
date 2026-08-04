/**
 * Top-of-page banner surfaced when the CLI's embedded wizard proxy
 * could not reach the NyxID backend on the last request — either it
 * timed out or the connection failed outright.
 *
 * Visually parallels `DisconnectBanner` (same destructive-tinted card
 * shape, same role="alert") but covers a different fault domain:
 *   - DisconnectBanner: browser ↔ CLI heartbeat failed.
 *   - UpstreamErrorBanner: CLI ↔ NyxID backend request failed.
 *
 * Both can be present at once and don't conflict. This banner is
 * non-blocking (the user can still edit / retry the form below it)
 * and dismissible — the inline `<ErrorLine />` at the submit button
 * keeps the focused context, while the banner provides the page-
 * level "something went wrong" signal that the issue (#711) asks for.
 */

import { Clock, WifiOff, X } from "lucide-react";

export type UpstreamErrorKind = "timeout" | "unreachable";

export interface UpstreamErrorBannerProps {
  readonly kind: UpstreamErrorKind;
  readonly onDismiss: () => void;
}

export function UpstreamErrorBanner({
  kind,
  onDismiss,
}: UpstreamErrorBannerProps) {
  const Icon = kind === "timeout" ? Clock : WifiOff;
  const title =
    kind === "timeout"
      ? "Request to NyxID timed out"
      : "NyxID backend unreachable";
  const body =
    kind === "timeout"
      ? "The page took too long to reach the NyxID backend. No changes were made. Try again from the form below — or close this tab and re-run the command in your terminal."
      : "Couldn't reach the NyxID backend on the last attempt. No changes were made. Check your network, then try again from the form below — or close this tab and re-run the command in your terminal.";

  return (
    <div
      role="alert"
      aria-live="polite"
      className="mb-4 flex items-start gap-3 rounded-lg border border-destructive/50 bg-destructive/10 px-4 py-3 text-[12px] text-foreground"
    >
      <Icon
        className="mt-0.5 h-4 w-4 shrink-0 text-destructive"
        aria-hidden
      />
      <div className="flex flex-1 flex-col gap-1">
        <p className="font-medium">{title}</p>
        <p className="text-muted-foreground">{body}</p>
      </div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss"
        className="rounded p-1 text-muted-foreground hover:bg-destructive/10 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-destructive"
      >
        <X className="h-3.5 w-3.5" aria-hidden />
      </button>
    </div>
  );
}
