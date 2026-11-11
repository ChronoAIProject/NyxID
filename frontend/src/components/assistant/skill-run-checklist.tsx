import { Check, Circle, Loader2, MessageCircleQuestion, SkipForward, X } from "lucide-react";
import { STATUS_CARD_MAX_W } from "@/lib/assistant/banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export type SkillRunStepStatus = "pending" | "running" | "needs_you" | "done" | "failed" | "skipped";

/** A step of the skill a running step uses, shown one level deep. */
export interface SkillRunSubstep {
  readonly id: string;
  readonly title: string;
  readonly status: SkillRunStepStatus;
}

export interface SkillRunStep {
  readonly id: string;
  readonly title: string;
  readonly status: SkillRunStepStatus;
  readonly uses?: { readonly name: string; readonly version: string } | null;
  /** What the step is doing now, what it produced, or what it needs from you. */
  readonly detail?: string | null;
  /** The used skill's own steps, read when this step starts. */
  readonly substeps?: readonly SkillRunSubstep[];
}

export interface SkillRunChecklistProps {
  readonly name: string;
  readonly version: string;
  readonly kind: "skill" | "skillset";
  readonly steps: readonly SkillRunStep[];
  /** Elapsed time of the running step, already formatted. */
  readonly elapsed?: string;
  readonly onStop?: () => void;
}

const ICON = "size-3.5 shrink-0";

function StepIcon({ status, small = false }: { readonly status: SkillRunStepStatus; readonly small?: boolean }) {
  const size = small ? "size-3 shrink-0" : ICON;
  switch (status) {
    case "running":
      return <Loader2 aria-label="Running" className={cn(size, "animate-spin text-nyx-secondary-400")} />;
    case "done":
      return <Check aria-label="Done" className={cn(size, "text-success")} />;
    case "needs_you":
      return <MessageCircleQuestion aria-label="Needs you" className={cn(size, "text-warning")} />;
    case "failed":
      return <X aria-label="Failed" className={cn(size, "text-destructive")} />;
    case "skipped":
      return <SkipForward aria-label="Skipped" className={cn(size, "text-text-tertiary")} />;
    case "pending":
    default:
      return <Circle aria-label="Not started" className={cn(size, "text-text-tertiary/60")} />;
  }
}

/**
 * The step the run is on: a running or waiting step, else the next pending
 * one. With no live step, a failed step means the run stopped there; `-1`
 * when the run has ended.
 */
function currentStep(steps: readonly SkillRunStep[]): number {
  const live = steps.findIndex((step) => step.status === "running" || step.status === "needs_you");
  if (live !== -1) return live;
  if (steps.some((step) => step.status === "failed")) return -1;
  return steps.findIndex((step) => step.status === "pending");
}

/**
 * The live checklist of a skill run: the skill's own Approach steps, with the
 * current one marked. Titles come from the skill's outline; details are
 * NyxBot's short progress notes. It shows progress only: confirmations still
 * arrive as their own cards.
 */
export function SkillRunChecklist({ name, version, kind, steps, elapsed, onStop }: SkillRunChecklistProps) {
  const done = steps.filter((step) => step.status === "done" || step.status === "skipped").length;
  const current = currentStep(steps);
  const finished = current === -1;
  const failed = steps.findIndex((step) => step.status === "failed");
  const stopped = finished && failed !== -1;
  const active = finished ? undefined : steps[current];
  const heading = stopped
    ? "Stopped"
    : finished
      ? "Done"
      : kind === "skillset"
        ? "Running skillset"
        : "Running skill";
  const label = stopped
    ? `Step ${failed + 1} of ${steps.length} failed`
    : finished
      ? `Finished · ${steps.length} steps`
      : `Step ${current + 1} of ${steps.length}`;
  return (
    <section
      aria-label={`${name} run`}
      className={`${STATUS_CARD_MAX_W} space-y-3 rounded-xl border border-border/50 bg-card p-4`}
    >
      <div className="space-y-2">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 space-y-1">
            <p className="type-group-label uppercase tracking-[1.5px] text-text-tertiary">{heading}</p>
            <p className="type-card-title break-words text-foreground">
              {name} <span className="font-mono font-normal text-text-tertiary">{version}</span>
            </p>
          </div>
          <span
            role="status"
            className={cn("type-secondary shrink-0 pt-0.5", stopped ? "text-destructive" : "text-muted-foreground")}
          >
            {label}
            {active?.status === "running" && elapsed ? ` · ${elapsed}` : ""}
          </span>
        </div>
        <div
          role="progressbar"
          aria-label="Steps done"
          aria-valuemin={0}
          aria-valuemax={steps.length}
          aria-valuenow={done}
          className="h-1 overflow-hidden rounded-full bg-overlay"
        >
          <div
            className={cn(
              "h-full rounded-full transition-[width] duration-500",
              stopped ? "bg-destructive" : finished ? "bg-success" : "nyx-gradient-vivid",
            )}
            style={{ width: `${steps.length ? (done / steps.length) * 100 : 0}%` }}
          />
        </div>
      </div>

      <ol aria-label="Steps" className="space-y-1">
        {steps.map((step) => {
          const live = step.status === "running";
          const asking = step.status === "needs_you";
          return (
            <li
              key={step.id}
              aria-current={live || asking ? "step" : undefined}
              className={cn(
                "flex gap-2.5 rounded-lg px-2 py-1.5",
                live && "bg-overlay/50",
                asking && "bg-warning/[0.06]",
              )}
            >
              <span className="mt-[3px]">
                <StepIcon status={step.status} />
              </span>
              <div className="min-w-0 flex-1 space-y-1">
                <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                  <span
                    className={cn(
                      "type-row-title break-words",
                      live ? "skill-step-shimmer" : step.status === "pending" ? "text-text-tertiary" : "text-foreground",
                    )}
                  >
                    {step.title}
                  </span>
                  {asking ? <Badge variant="warning">Needs you</Badge> : null}
                  {step.uses ? (
                    <span className="type-secondary font-mono text-text-tertiary">{step.uses.name}</span>
                  ) : null}
                </div>
                {step.detail ? (
                  <p className={cn("type-secondary break-words", asking ? "text-foreground" : "text-muted-foreground")}>
                    {step.detail}
                  </p>
                ) : null}
                {(live || asking) && step.substeps?.length ? (
                  <ul aria-label={`${step.uses?.name ?? step.title} steps`} className="space-y-1 border-l border-hairline py-0.5 pl-2.5">
                    {step.substeps.map((substep) => (
                      <li key={substep.id} className="flex min-w-0 items-center gap-1.5">
                        <StepIcon status={substep.status} small />
                        <span
                          className={cn(
                            "type-secondary min-w-0 break-words",
                            substep.status === "running"
                              ? "skill-step-shimmer"
                              : substep.status === "pending"
                                ? "text-text-tertiary"
                                : "text-muted-foreground",
                          )}
                        >
                          {substep.title}
                        </span>
                      </li>
                    ))}
                  </ul>
                ) : null}
              </div>
            </li>
          );
        })}
      </ol>

      {!finished && onStop ? (
        <div className="flex justify-end">
          <Button size="sm" variant="outline" onClick={onStop}>
            Stop
          </Button>
        </div>
      ) : null}
    </section>
  );
}
