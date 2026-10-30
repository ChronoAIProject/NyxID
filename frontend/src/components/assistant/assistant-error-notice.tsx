import { useState } from "react";
import { X } from "lucide-react";

/**
 * A failure the person may need to act on, kept outside the transcript
 * scroller. Dismissing hides that message only; a different one shows again.
 * `docked` sits it on the chat column just above the composer instead of
 * across the top of the page.
 */
export function AssistantErrorNotice({
  message,
  docked = false,
}: {
  readonly message?: string;
  readonly docked?: boolean;
}) {
  const [dismissed, setDismissed] = useState<string>();
  if (!message || dismissed === message) return null;
  return (
    <div
      className={docked ? "px-4 pb-2 sm:px-6" : "shrink-0 px-4 pt-3 sm:px-6"}
      style={docked ? { width: "calc(100% - var(--assistant-content-inset, 0px))" } : undefined}
    >
      {/* Docked over the transcript, the tint sits on an opaque surface. */}
      <div
        className={`mx-auto w-full rounded-xl ${
          docked ? "max-w-[var(--chat-content-width,758px)] bg-background shadow-sm" : "max-w-[758px]"
        }`}
      >
        <div
          role="alert"
          className="flex items-start gap-2 rounded-xl border border-destructive/25 bg-destructive/5 py-2.5 pl-4 pr-2 text-12 text-destructive"
        >
          <p className="min-w-0 flex-1 whitespace-pre-line break-words py-0.5">{message}</p>
          <button
            type="button"
            aria-label="Dismiss"
            onClick={() => setDismissed(message)}
            className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-destructive/70 transition-colors hover:bg-destructive/10 hover:text-destructive"
          >
            <X aria-hidden="true" className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
}
