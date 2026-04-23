/**
 * Context-aware footer for the shared wizard shell.
 *
 * Mode A (local wizard) shows "Served locally from 127.0.0.1:<port> ·
 * Nothing leaves your machine" — a trust anchor that tells the user the
 * page is served by the CLI and that credentials only live in their
 * process. Mode B (remote pairing on `/cli/pair`) is served by the NyxID
 * frontend, so that exact copy would be misleading. The copy here
 * branches on `context` to tell each user the truth about their
 * connection.
 */

export interface WizardFooterProps {
  readonly context: "local" | "pair"
  /** Optional origin for Mode A — the CLI server's bound address. */
  readonly localOrigin?: string
}

export function WizardFooter({ context, localOrigin }: WizardFooterProps) {
  if (context === "local") {
    return (
      <footer className="mt-6 flex flex-wrap gap-2 border-t border-border pt-4 text-[13px] text-muted-foreground">
        <span>
          Served locally from{" "}
          <code className="rounded bg-muted/60 px-1.5 py-0.5 font-mono text-xs">
            {localOrigin ?? window.location.host}
          </code>
        </span>
        <span>· Nothing leaves your machine</span>
      </footer>
    )
  }

  return (
    <footer className="mt-6 flex flex-wrap gap-2 border-t border-border pt-4 text-[13px] text-muted-foreground">
      <span>Pairing with a remote CLI</span>
      <span>· Secrets never travel back through the pairing channel</span>
    </footer>
  )
}
