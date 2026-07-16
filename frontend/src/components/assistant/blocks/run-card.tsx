import { Check, Clock3, Loader2, SkipForward, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { RunContentBlock, RunStepStatus } from "@/types/assistant";

function StepIcon({ status }: { readonly status: RunStepStatus }) {
  const className = "h-3 w-3";
  switch (status) {
    case "done":
      return <Check className={className} />;
    case "active":
      return <Loader2 className={`${className} animate-spin`} />;
    case "waiting":
      return <Clock3 className={className} />;
    case "failed":
      return <X className={className} />;
    case "skipped":
      return <SkipForward className={className} />;
  }
}

function stepTone(status: RunStepStatus): string {
  if (status === "done") return "border-success/30 bg-success/10 text-success";
  if (status === "failed") {
    return "border-destructive/30 bg-destructive/10 text-destructive";
  }
  if (status === "active") return "border-info/30 bg-info/10 text-info";
  return "border-hairline bg-overlay text-text-tertiary";
}

export function RunCard({ block }: { readonly block: RunContentBlock }) {
  const terminal = block.state === "completed" || block.state === "failed";
  return (
    <section className="rounded-xl border border-border/70 bg-card p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 font-mono text-[10px] font-semibold uppercase text-muted-foreground">
          {terminal ? (
            block.state === "completed" ? (
              <Check className="h-3.5 w-3.5 text-success" />
            ) : (
              <X className="h-3.5 w-3.5 text-destructive" />
            )
          ) : (
            <Loader2 className="h-3.5 w-3.5 animate-spin text-info" />
          )}
          {block.title} - {String(block.steps_complete)} of{" "}
          {String(block.steps_total)} steps complete
        </div>
        <Badge
          variant={
            block.state === "completed"
              ? "success"
              : block.state === "failed"
                ? "destructive"
                : "warning"
          }
        >
          {block.state.replaceAll("_", " ")}
        </Badge>
      </div>

      <ol className="space-y-3">
        {block.steps.map((step) => (
          <li key={step.index} className="flex gap-3">
            <span
              className={`mt-0.5 flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-md border ${stepTone(step.status)}`}
            >
              <StepIcon status={step.status} />
            </span>
            <div className="min-w-0">
              <p className="break-words font-mono text-[11px] leading-relaxed text-foreground">
                {step.label}
              </p>
              <p className="mt-0.5 text-[10px] leading-relaxed text-text-tertiary">
                {step.meta}
              </p>
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}
