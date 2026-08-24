import { Check, RotateCcw, ShieldAlert, SkipForward, StopCircle, X } from "lucide-react";
import { ActionCard } from "@/components/assistant/blocks/action-card";
import { InputCard } from "@/components/assistant/blocks/input-card";
import { TaskPlanCard } from "@/components/assistant/blocks/task-plan-card";
import { Button } from "@/components/ui/button";
import { actionSummaryBlock } from "@/lib/assistant/chat-action-presentation";
import type {
  ChatActorProjection,
  ChatActorStep,
  ChatPendingInput,
} from "@/lib/assistant/chat-actor-state";
import type { ChatInputAnswer } from "@/lib/assistant/chat-api";
import type { ChatPlanGate } from "@/lib/assistant/chat-task-plan";
import type { ActionReport } from "@/schemas/assistant-actions";
import type {
  InputCardContentBlock,
  TaskPlanContentBlock,
} from "@/types/assistant";

function inputBlock(input: ChatPendingInput, stateVersion: number): InputCardContentBlock {
  return {
    type: "input_card",
    block_id: `current-input:${input.requestId}`,
    request_id: input.requestId,
    prompt: input.prompt,
    options: input.options.map((option) => ({
      option_id: option.optionId,
      label: option.label,
      description: option.description,
    })),
    allow_free_text: input.allowFreeText,
    multi_select: input.multiSelect,
    state_version: stateVersion,
    status: "pending",
  };
}

export function ChatActorControls({
  projection,
  disabled,
  actionOverrides,
  onResolveInput,
  onResolveApproval,
  onResolvePlan,
  onStop,
  onControlStep,
  onActionProgress,
  onBlockAction,
  onResolveAction,
}: {
  readonly projection: ChatActorProjection | null;
  readonly disabled: boolean;
  readonly actionOverrides: ReadonlyMap<
    string,
    { readonly status?: string; readonly note?: string }
  >;
  readonly onResolveInput: (
    answer: ChatInputAnswer,
    input: ChatPendingInput,
  ) => Promise<void>;
  readonly onResolveApproval: (
    requestId: string,
    approved: boolean,
  ) => Promise<void>;
  readonly onResolvePlan: (
    confirmed: boolean,
    gate: ChatPlanGate,
  ) => Promise<void>;
  readonly onStop: () => Promise<void>;
  readonly onControlStep: (
    type: "step.retry" | "step.skip",
    step: ChatActorStep,
  ) => Promise<void>;
  readonly onActionProgress: (actionRequestId: string, active: boolean) => void;
  readonly onBlockAction: (actionRequestId: string, note: string) => void;
  readonly onResolveAction: (report: ActionReport) => Promise<void>;
}) {
  if (!projection) return null;
  const actions = [...projection.actions.values()];
  const controllableSteps = [...projection.steps.values()].filter(
    (step) => step.availableActions.retry || step.availableActions.skip,
  );
  const pendingApproval = projection.pendingApproval;
  const hasControls = Boolean(
    projection.task ||
      projection.pendingInput ||
      pendingApproval ||
      actions.length ||
      controllableSteps.length,
  );
  if (!hasControls) return null;

  return (
    <section aria-label="Actor controls" className="space-y-3">
      {projection.task ? (
        <TaskPlanCard
          block={
            {
              type: "task_plan",
              block_id: `current-task:${projection.task.taskId}`,
              state_version: projection.stateVersion,
              progress_sequence: projection.progressSequence,
              plan: projection.task,
            } as unknown as TaskPlanContentBlock
          }
          onStop={onStop}
          onRetry={(stepId) => {
            const step = projection.steps.get(stepId);
            return step ? onControlStep("step.retry", step) : Promise.resolve();
          }}
          onSkip={(stepId) => {
            const step = projection.steps.get(stepId);
            return step ? onControlStep("step.skip", step) : Promise.resolve();
          }}
          onResolve={(confirmed) => {
            const gate = projection.task?.gate;
            return gate ? onResolvePlan(confirmed, gate) : Promise.resolve();
          }}
        />
      ) : null}

      {projection.pendingInput ? (
        <InputCard
          block={inputBlock(projection.pendingInput, projection.stateVersion)}
          onResolve={(answer) =>
            onResolveInput(answer, projection.pendingInput as ChatPendingInput)
          }
        />
      ) : null}

      {pendingApproval ? (
        <section className="overflow-hidden rounded-lg border border-warning/30 bg-warning/[0.05]">
          <div className="flex items-start gap-2 border-b border-warning/20 px-3 py-2.5">
            <ShieldAlert className="mt-0.5 h-4 w-4 text-warning" />
            <div className="min-w-0 flex-1">
              <h3 className="text-[12px] font-semibold text-foreground">Approval required</h3>
              <p className="mt-0.5 break-words text-[11px] text-muted-foreground">
                {pendingApproval.toolName || pendingApproval.action || pendingApproval.approvalRequestId}
              </p>
            </div>
          </div>
          <div className="flex justify-end gap-2 px-3 py-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={disabled}
              onClick={() => void onResolveApproval(pendingApproval.approvalRequestId, false)}
            >
              <X /> Reject
            </Button>
            <Button
              type="button"
              size="sm"
              disabled={disabled}
              onClick={() => void onResolveApproval(pendingApproval.approvalRequestId, true)}
            >
              <Check /> Approve
            </Button>
          </div>
        </section>
      ) : null}

      {controllableSteps.map((step) => (
        <section key={step.stepId} className="rounded-lg border border-border bg-card px-3 py-2.5">
          <div className="text-[12px] font-medium text-foreground">
            {step.description || step.stepId}
          </div>
          <div className="mt-2 flex gap-2">
            {step.availableActions.retry ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={disabled}
                onClick={() => void onControlStep("step.retry", step)}
              >
                <RotateCcw /> Retry
              </Button>
            ) : null}
            {step.availableActions.skip ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={disabled}
                onClick={() => void onControlStep("step.skip", step)}
              >
                <SkipForward /> Skip
              </Button>
            ) : null}
            {step.availableActions.stop ? (
              <Button
                type="button"
                size="sm"
                variant="destructive"
                disabled={disabled}
                onClick={() => void onStop()}
              >
                <StopCircle /> Stop
              </Button>
            ) : null}
          </div>
        </section>
      ))}

      {actions.map((summary) => {
        const block = actionSummaryBlock(
          summary,
          actionOverrides.get(summary.actionRequestId),
        );
        return (
          <ActionCard
            key={summary.actionRequestId}
            block={block}
            onProgress={(_, active) => onActionProgress(summary.actionRequestId, active)}
            onBlock={(_, note) => onBlockAction(summary.actionRequestId, note)}
            onResolve={onResolveAction}
          />
        );
      })}
    </section>
  );
}
