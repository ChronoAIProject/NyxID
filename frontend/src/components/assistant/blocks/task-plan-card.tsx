import { useState } from "react";
import {
  Check,
  Circle,
  CircleAlert,
  CircleStop,
  Clock3,
  Loader2,
  Minus,
  Redo2,
  ShieldCheck,
  SkipForward,
  Square,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import type {
  ChatActorStep,
  ChatExternalEffect,
  ChatTaskPlan,
  ChatTaskStepStatus,
} from "@/lib/assistant/chat-task-plan";

interface TaskPlanCardBlock {
  readonly type: "task_plan";
  readonly block_id: string;
  readonly state_version: number;
  readonly progress_sequence: number;
  readonly plan: ChatTaskPlan;
}

type TaskControl = "stop" | `retry:${string}` | `skip:${string}`;

const STATUS_STYLE: Record<ChatTaskStepStatus, string> = {
  planned: "text-text-tertiary",
  waiting: "text-warning",
  running: "text-nyx-secondary-400",
  done: "text-success",
  failed: "text-destructive",
  skipped: "text-text-tertiary",
  cancelled: "text-text-tertiary",
  uncertain: "text-warning",
};

function words(value: string): string {
  return value.replaceAll("_", " ");
}

function StepIcon({ status }: { readonly status: ChatTaskStepStatus }) {
  const className = `h-3.5 w-3.5 shrink-0 ${STATUS_STYLE[status]}`;
  switch (status) {
    case "running":
      return <Loader2 className={`${className} animate-spin`} />;
    case "done":
      return <Check className={className} />;
    case "failed":
      return <X className={className} />;
    case "waiting":
      return <Clock3 className={className} />;
    case "skipped":
      return <SkipForward className={className} />;
    case "cancelled":
      return <CircleStop className={className} />;
    case "uncertain":
      return <CircleAlert className={className} />;
    case "planned":
    default:
      return <Circle className={className} />;
  }
}

function effectStyle(effect: ChatExternalEffect): string {
  switch (effect) {
    case "confirmed":
      return "border-success/25 bg-success/[0.07] text-success";
    case "may_have_changed":
      return "border-warning/30 bg-warning/[0.08] text-warning";
    case "not_applied":
      return "border-border bg-overlay text-muted-foreground";
    case "not_started":
    default:
      return "border-hairline bg-transparent text-text-tertiary";
  }
}

function StepDetails({ step }: { readonly step: ChatActorStep }) {
  const sourceDetail =
    step.source.kind === "tool" && step.source.serviceSlug
      ? `${step.source.label} / ${step.source.serviceSlug}`
      : step.source.label;
  const operation = step.operation;
  const operationDetail = operation
    ? [
        operation.kind,
        operation.phase,
        operation.operationId,
        operation.operationGeneration
          ? `generation ${String(operation.operationGeneration)}`
          : null,
      ]
        .filter(Boolean)
        .join(" / ")
    : "";

  return (
    <div className="min-w-0 flex-1">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
        <span className="min-w-0 break-words text-[12px] font-medium text-foreground">
          {step.description}
        </span>
        <span
          className={`text-[10px] font-medium ${STATUS_STYLE[step.status]}`}
        >
          {words(step.status)}
        </span>
        <span
          className={`rounded border px-1.5 py-0.5 text-[9px] font-medium ${effectStyle(step.externalEffect)}`}
        >
          {words(step.externalEffect)}
        </span>
      </div>
      <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-text-tertiary">
        <span>{sourceDetail}</span>
        {step.estimate ? <span>~{step.estimate.seconds}s</span> : null}
        {step.addedInPlanRevision ? (
          <span>revision {step.addedInPlanRevision}</span>
        ) : null}
      </div>
      {operationDetail ? (
        <div className="mt-1.5 break-words rounded bg-overlay px-2 py-1 font-mono text-[9px] text-muted-foreground">
          {operationDetail}
        </div>
      ) : null}
      {step.source.kind === "condition" ? (
        <div className="mt-1.5 text-[10px] text-muted-foreground">
          {step.source.condition.observedValue} gte{" "}
          {step.source.condition.effectiveThreshold}
          {" / "}
          {step.source.condition.thresholdOrigin}
          {" / "}
          {step.source.condition.outcome}
        </div>
      ) : null}
      {step.guard ? (
        <div className="mt-1 text-[10px] text-muted-foreground">
          Guard {step.guard.conditionStepId}: {step.guard.requiredOutcome}
        </div>
      ) : null}
      {step.substeps.length > 0 ? (
        <ul className="mt-2 space-y-1 border-l border-hairline pl-2.5">
          {step.substeps.map((substep) => (
            <li
              key={substep.substepId}
              className="flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground"
            >
              {substep.status === "done" ? (
                <Check className="h-3 w-3 shrink-0 text-success" />
              ) : substep.status === "failed" ? (
                <X className="h-3 w-3 shrink-0 text-destructive" />
              ) : (
                <Loader2 className="h-3 w-3 shrink-0 animate-spin text-nyx-secondary-400" />
              )}
              <span className="min-w-0 break-words">{substep.title}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {step.approvalObservation ? (
        <div className="mt-1.5 flex items-center gap-1.5 text-[10px] text-muted-foreground">
          <ShieldCheck className="h-3 w-3 shrink-0" />
          {step.approvalObservation.decisionMode} /{" "}
          {words(step.approvalObservation.receiptStatus)}
        </div>
      ) : null}
      {step.safeMessage ? (
        <p className="mt-1.5 break-words text-[10px] text-destructive">
          {step.safeMessage}
        </p>
      ) : null}
    </div>
  );
}

export function TaskPlanCard({
  block,
  disabled = false,
  onStop,
  onRetry,
  onSkip,
}: {
  readonly block: TaskPlanCardBlock;
  readonly disabled?: boolean;
  readonly onStop: () => Promise<void>;
  readonly onRetry: (stepId: string) => Promise<void>;
  readonly onSkip: (stepId: string) => Promise<void>;
}) {
  const [pending, setPending] = useState<TaskControl | null>(null);
  const plan = block.plan;
  const canStop = plan.steps.some((step) => step.availableActions.stop);

  async function run(control: TaskControl, action: () => Promise<void>) {
    if (disabled || pending) return;
    setPending(control);
    try {
      await action();
    } finally {
      setPending(null);
    }
  }

  return (
    <section
      aria-label="Task plan"
      className="overflow-hidden rounded-lg border border-border bg-card shadow-sm"
    >
      <div className="flex flex-wrap items-start justify-between gap-2 border-b border-border bg-overlay/50 px-3 py-2.5">
        <div className="min-w-0 flex-1">
          <h3 className="break-words text-[12px] font-semibold text-foreground">
            {plan.title}
          </h3>
          <p className="mt-0.5 text-[10px] text-text-tertiary">
            Revision {plan.planRevision} / state {block.state_version} /
            sequence {block.progress_sequence}
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-1.5">
          <span className="rounded border border-border bg-background px-1.5 py-0.5 text-[9px] font-medium text-muted-foreground">
            {words(plan.status)}
          </span>
          {plan.gate ? (
            <span className="rounded border border-warning/25 bg-warning/[0.07] px-1.5 py-0.5 text-[9px] font-medium text-warning">
              {plan.gate.mode} / {plan.gate.status ?? "ready"}
            </span>
          ) : null}
        </div>
      </div>
      {plan.gate?.reason ? (
        <p className="border-b border-border px-3 py-2 text-[10px] leading-relaxed text-muted-foreground">
          {plan.gate.reason}
        </p>
      ) : null}
      <ol className="divide-y divide-border">
        {plan.steps.map((step) => (
          <li
            key={step.stepId}
            className="grid grid-cols-[18px_minmax(0,1fr)] gap-2.5 px-3 py-2.5"
          >
            <div className="flex flex-col items-center gap-1 pt-0.5">
              <StepIcon status={step.status} />
              <span className="font-mono text-[9px] text-text-tertiary">
                {step.order}
              </span>
            </div>
            <div className="min-w-0">
              <StepDetails step={step} />
              {step.availableActions.retry || step.availableActions.skip ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {step.availableActions.retry ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={disabled || pending !== null}
                      aria-label={`Retry ${step.description}`}
                      onClick={() =>
                        void run(`retry:${step.stepId}`, () =>
                          onRetry(step.stepId),
                        )
                      }
                    >
                      {pending === `retry:${step.stepId}` ? (
                        <Loader2 className="animate-spin" />
                      ) : (
                        <Redo2 />
                      )}
                      Retry
                    </Button>
                  ) : null}
                  {step.availableActions.skip ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={disabled || pending !== null}
                      aria-label={`Skip ${step.description}`}
                      onClick={() =>
                        void run(`skip:${step.stepId}`, () =>
                          onSkip(step.stepId),
                        )
                      }
                    >
                      {pending === `skip:${step.stepId}` ? (
                        <Loader2 className="animate-spin" />
                      ) : (
                        <Minus />
                      )}
                      Skip
                    </Button>
                  ) : null}
                </div>
              ) : null}
            </div>
          </li>
        ))}
      </ol>
      {canStop ? (
        <div className="flex justify-end border-t border-border px-3 py-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={disabled || pending !== null}
            onClick={() => void run("stop", onStop)}
          >
            {pending === "stop" ? (
              <Loader2 className="animate-spin" />
            ) : (
              <Square className="fill-current" />
            )}
            Stop task
          </Button>
        </div>
      ) : null}
    </section>
  );
}
