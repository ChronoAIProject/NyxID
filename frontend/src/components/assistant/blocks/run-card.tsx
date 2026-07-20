import { Check, Clock3, Loader2, X } from "lucide-react";
import type { RunContentBlock } from "@/types/assistant";

const TERMINAL_STATES: ReadonlySet<RunContentBlock["state"]> = new Set([
  "completed",
  "failed",
  "cancelled",
]);

/**
 * Tool activity is a transient whisper, not a card (DESIGN.md: compact,
 * color is earned). While the run streams, only in-flight steps render — a
 * slim 11px mono line per running/waiting tool; each disappears with its
 * TOOL_CALL_END. When the run settles, everything collapses to a single
 * micro-summary line (or a one-line failure with its redacted reason).
 */
export function RunCard({ block }: { readonly block: RunContentBlock }) {
  if (!TERMINAL_STATES.has(block.state)) {
    const inflight = block.steps.filter(
      (step) => step.status === "active" || step.status === "waiting",
    );
    if (inflight.length === 0) return null;
    return (
      <div className="space-y-1 py-0.5" role="status" aria-label="Tool activity">
        {inflight.map((step) => (
          <div
            key={step.index}
            className="flex items-center gap-2 text-[11px] text-muted-foreground"
          >
            {step.status === "active" ? (
              <Loader2 className="h-3 w-3 shrink-0 animate-spin text-text-tertiary" />
            ) : (
              <Clock3 className="h-3 w-3 shrink-0 text-warning" />
            )}
            <span className="truncate font-mono">{step.label}</span>
            {step.status === "waiting" && (
              <span className="shrink-0 text-text-tertiary">
                waiting for approval
              </span>
            )}
          </div>
        ))}
      </div>
    );
  }

  const failed = block.steps.find((step) => step.status === "failed");
  if (failed) {
    return (
      <div className="flex items-center gap-2 py-0.5 text-[11px] text-muted-foreground">
        <X className="h-3 w-3 shrink-0 text-destructive" />
        <span className="truncate font-mono">{failed.label}</span>
        {failed.meta && (
          <span className="truncate text-text-tertiary">{failed.meta}</span>
        )}
      </div>
    );
  }

  const done = block.steps.filter((step) => step.status === "done");
  if (done.length === 0) return null;
  return (
    <div className="flex items-center gap-2 py-0.5 text-[11px] text-text-tertiary">
      <Check className="h-3 w-3 shrink-0 text-success/70" />
      <span className="truncate font-mono">
        {done.map((step) => step.label).join(" · ")}
      </span>
    </div>
  );
}
