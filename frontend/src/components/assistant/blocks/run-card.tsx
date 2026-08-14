import { useState } from "react";
import { Check, ChevronRight, Clock3, Loader2, Minus, X } from "lucide-react";
import type { RunContentBlock, RunStepStatus } from "@/types/assistant";

const TERMINAL_STATES: ReadonlySet<RunContentBlock["state"]> = new Set([
  "completed",
  "failed",
  "cancelled",
]);

/**
 * Tool activity persists in the thread as an always-visible ledger (chat UI
 * refresh, 2026-07-22): every step keeps its status icon and label so the
 * reader can see what tools ran. TOOL_CALL_END only settles a step to
 * done/failed — it never removes the step and never ends the run (the run
 * closes on RUN_FINISHED), so a finished tool call stays on screen instead of
 * vanishing. The ledger defaults to expanded (tool names shown) and can be
 * collapsed to a one-line summary. Supersedes the earlier "transient whisper"
 * treatment where steps disappeared on completion.
 */
function StepIcon({ status }: { readonly status: RunStepStatus }) {
  switch (status) {
    case "active":
      return (
        <Loader2 className="h-3 w-3 shrink-0 animate-spin text-nyx-secondary-400" />
      );
    case "waiting":
      return <Clock3 className="h-3 w-3 shrink-0 text-warning" />;
    case "failed":
      return <X className="h-3 w-3 shrink-0 text-destructive" />;
    case "skipped":
      return <Minus className="h-3 w-3 shrink-0 text-text-tertiary" />;
    case "done":
    default:
      return <Check className="h-3 w-3 shrink-0 text-success/70" />;
  }
}

export function RunCard({ block }: { readonly block: RunContentBlock }) {
  const isTerminal = TERMINAL_STATES.has(block.state);
  // Default expanded so a completed tool call keeps showing what ran; the
  // reader can collapse it. Never auto-collapse on completion — that reads as
  // the tool call having "ended"/vanished.
  const [open, setOpen] = useState(true);

  const steps = block.steps.filter((step) => step.status !== "skipped");
  // Nothing worth persisting once the run has settled without any real step.
  if (steps.length === 0 && isTerminal) return null;

  const failed = steps.some((step) => step.status === "failed");
  const doneCount = steps.filter((step) => step.status === "done").length;

  let headerIcon;
  let headerLabel;
  if (!isTerminal) {
    headerIcon = (
      <Loader2 className="h-3 w-3 shrink-0 animate-spin text-nyx-secondary-400" />
    );
    headerLabel = block.title || "Working…";
  } else if (failed) {
    headerIcon = <X className="h-3 w-3 shrink-0 text-destructive" />;
    headerLabel = block.title || "Tool run failed";
  } else {
    headerIcon = <Check className="h-3 w-3 shrink-0 text-success/70" />;
    headerLabel =
      block.title || `${String(doneCount)} tool${doneCount === 1 ? "" : "s"} used`;
  }

  return (
    <div
      className="rounded-lg border border-hairline bg-overlay/40"
      role={isTerminal ? undefined : "status"}
      aria-label={isTerminal ? undefined : "Tool activity"}
    >
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[11px] text-muted-foreground transition-colors hover:bg-white/[0.02]"
      >
        <ChevronRight
          className={`h-3 w-3 shrink-0 text-text-tertiary transition-transform ${
            open ? "rotate-90" : ""
          }`}
        />
        {headerIcon}
        <span className="truncate font-medium">{headerLabel}</span>
        {!isTerminal && block.steps_total > 0 ? (
          <span className="ml-auto shrink-0 font-mono tabular-nums text-text-tertiary">
            {block.steps_complete}/{block.steps_total}
          </span>
        ) : null}
      </button>
      {open && steps.length > 0 ? (
        <div className="space-y-1.5 border-t border-hairline/70 px-2.5 py-2">
          {steps.map((step) => (
            <div key={step.index} className="flex items-start gap-2 text-[11px]">
              <span className="mt-px">
                <StepIcon status={step.status} />
              </span>
              <div className="min-w-0 flex-1">
                <span className="block truncate font-mono text-muted-foreground">
                  {step.label}
                </span>
                {step.meta ? (
                  <span className="block truncate text-text-tertiary">
                    {step.meta}
                  </span>
                ) : null}
                {step.result_preview ? (
                  <details className="mt-1 text-text-tertiary">
                    <summary className="cursor-pointer select-none text-[10px] font-medium hover:text-muted-foreground">
                      Result preview
                    </summary>
                    <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap break-words border-l border-hairline pl-2 font-mono text-[10px] leading-4 text-muted-foreground">
                      {step.result_preview}
                    </pre>
                  </details>
                ) : null}
                {step.status === "waiting" ? (
                  <span className="block text-text-tertiary">
                    waiting for approval
                  </span>
                ) : null}
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
