import { useState } from "react";
import {
  Check,
  CircleStop,
  RotateCcw,
  Send,
  ShieldAlert,
  SkipForward,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  actorCan,
  type ActorProjection,
  type ActorRecord,
} from "@/lib/assistant/actor-state";
import type { InputAnswer } from "@/schemas/assistant-input";

interface ActorTaskPanelProps {
  readonly projection: ActorProjection;
  readonly busy: boolean;
  readonly onResolvePlan: (confirmed: boolean) => Promise<void>;
  readonly onResolveInput: (
    requestId: string,
    answer: InputAnswer,
  ) => Promise<void>;
  readonly onResolveApproval: (
    requestId: string,
    approved: boolean,
  ) => Promise<void>;
  readonly onStop: () => Promise<void>;
  readonly onSteer: (instruction: string) => Promise<void>;
  readonly onRetry: (stepId: string) => Promise<void>;
  readonly onSkip: (stepId: string) => Promise<void>;
}

function record(value: unknown): ActorRecord | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as ActorRecord)
    : null;
}

function text(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function optionsFrom(input: ActorRecord | null) {
  return (Array.isArray(input?.["options"]) ? input["options"] : []).flatMap(
    (value) => {
      const option = record(value);
      const optionId = text(option?.["optionId"]);
      const label = text(option?.["label"]);
      return optionId && label
        ? [{ optionId, label, description: text(option?.["description"]) }]
        : [];
    },
  );
}

interface ActorInputRequestProps {
  readonly pendingInput: ActorRecord;
  readonly requestId: string;
  readonly busy: boolean;
  readonly onResolve: (requestId: string, answer: InputAnswer) => Promise<void>;
}

function ActorInputRequest({
  pendingInput,
  requestId,
  busy,
  onResolve,
}: ActorInputRequestProps) {
  const options = optionsFrom(pendingInput);
  const allowFreeText = pendingInput["allowFreeText"] === true;
  const multiSelect = pendingInput["multiSelect"] === true;
  const [selectedOptions, setSelectedOptions] = useState<Set<string>>(new Set());
  const [inputText, setInputText] = useState("");

  const toggleOption = (optionId: string) => {
    setSelectedOptions((current) => {
      const next = new Set(multiSelect ? current : []);
      if (next.has(optionId)) next.delete(optionId);
      else next.add(optionId);
      return next;
    });
    setInputText("");
  };

  const submit = async () => {
    const answer: InputAnswer = selectedOptions.size
      ? { selectedOptionIds: [...selectedOptions] }
      : { freeText: inputText.trim() };
    await onResolve(requestId, answer);
  };

  return (
    <div className="mt-3 border-t border-border/60 pt-3">
      <p className="text-[12px] font-medium text-foreground">
        {text(pendingInput["prompt"]) || "Input required"}
      </p>
      {options.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-2">
          {options.map((option) => (
            <Button
              key={option.optionId}
              type="button"
              variant={selectedOptions.has(option.optionId) ? "primary" : "outline"}
              size="sm"
              title={option.description || undefined}
              aria-pressed={selectedOptions.has(option.optionId)}
              disabled={busy}
              onClick={() => toggleOption(option.optionId)}
            >
              {option.label}
            </Button>
          ))}
        </div>
      ) : null}
      <div className="mt-2 flex gap-2">
        {allowFreeText ? (
          <Input
            value={inputText}
            disabled={busy || selectedOptions.size > 0}
            maxLength={32_768}
            aria-label="Assistant input response"
            onChange={(event) => setInputText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void submit();
            }}
          />
        ) : null}
        <Button
          type="button"
          variant="primary"
          size="icon"
          aria-label="Submit input response"
          disabled={busy || (selectedOptions.size === 0 && !inputText.trim())}
          onClick={() => void submit()}
        >
          <Send />
        </Button>
      </div>
    </div>
  );
}

export function ActorTaskPanel({
  projection,
  busy,
  onResolvePlan,
  onResolveInput,
  onResolveApproval,
  onStop,
  onSteer,
  onRetry,
  onSkip,
}: ActorTaskPanelProps) {
  const task = projection.task;
  const gate = record(task?.["gate"]);
  const pendingInput = projection.pendingInput;
  const pendingApproval = projection.pendingApproval;
  const inputRequestId = text(pendingInput?.["requestId"]);
  const approvalRequestId = text(
    pendingApproval?.["approvalRequestId"] ?? pendingApproval?.["requestId"],
  );
  const [steering, setSteering] = useState("");

  const hasContent = Boolean(
    task ||
      pendingInput ||
      pendingApproval ||
      projection.actions.size > 0 ||
      projection.conflicts.length > 0,
  );
  if (!hasContent) return null;

  const taskStatus = text(task?.["status"] ?? projection.taskStatus) || "unknown";
  const gateCanResolve =
    text(gate?.["mode"]).toLowerCase() === "confirm" &&
    text(gate?.["status"]).toLowerCase() === "pending" &&
    Boolean(
      text(gate?.["requestId"]) &&
        text(task?.["taskId"]) &&
        text(task?.["planId"]),
    ) &&
    Number.isSafeInteger(task?.["planRevision"]) &&
    projection.stateVersion > 0;
  const approvalWithinGrant =
    text(pendingApproval?.["grantBoundary"]) === "within_grant";
  const submitSteering = async () => {
    const instruction = steering.trim();
    if (!instruction) return;
    await onSteer(instruction);
    setSteering("");
  };

  return (
    <section className="shrink-0 border-y border-border/70 bg-card/70">
      <div className="mx-auto max-h-[42vh] w-full max-w-[758px] overflow-y-auto px-4 py-3 sm:px-6">
        {task ? (
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-[11px] font-medium uppercase text-text-tertiary">
                Plan revision {String(task["planRevision"] ?? 1)}
              </p>
              <h2 className="truncate text-[13px] font-semibold text-foreground">
                {text(task["title"]) || text(task["safeMessage"]) || "Assistant task"}
              </h2>
            </div>
            <span className="shrink-0 text-[11px] font-medium text-text-secondary">
              {taskStatus}
            </span>
          </div>
        ) : null}

        {gateCanResolve ? (
          <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-border/60 pt-3">
            <span className="mr-auto text-[12px] text-text-secondary">
              {text(gate?.["reason"]) || "Plan confirmation required"}
            </span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => void onResolvePlan(false)}
            >
              <X /> Reject
            </Button>
            <Button
              type="button"
              variant="primary"
              size="sm"
              disabled={busy}
              onClick={() => void onResolvePlan(true)}
            >
              <Check /> Confirm
            </Button>
          </div>
        ) : null}

        {pendingInput && inputRequestId ? (
          <ActorInputRequest
            key={inputRequestId}
            pendingInput={pendingInput}
            requestId={inputRequestId}
            busy={busy}
            onResolve={onResolveInput}
          />
        ) : null}

        {pendingApproval && approvalRequestId ? (
          <div className="mt-3 border-t border-border/60 pt-3">
            <div className="flex items-start gap-2">
              <ShieldAlert className="mt-0.5 size-4 shrink-0 text-warning" />
              <div className="min-w-0 flex-1 text-[12px]">
                <p className="font-medium text-foreground">
                  {text(pendingApproval["action"] ?? pendingApproval["toolName"]) ||
                    "Approval required"}
                </p>
                {text(pendingApproval["target"]) ? (
                  <p className="truncate text-text-secondary">
                    {text(pendingApproval["target"])}
                  </p>
                ) : null}
              </div>
              {approvalWithinGrant ? (
                <div className="flex shrink-0 gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={busy || projection.stateVersion < 1}
                    onClick={() =>
                      void onResolveApproval(approvalRequestId, false)
                    }
                  >
                    <X /> Deny
                  </Button>
                  <Button
                    type="button"
                    variant="primary"
                    size="sm"
                    disabled={busy || projection.stateVersion < 1}
                    onClick={() =>
                      void onResolveApproval(approvalRequestId, true)
                    }
                  >
                    <Check /> Approve
                  </Button>
                </div>
              ) : null}
            </div>
          </div>
        ) : null}

        {projection.steps.size > 0 ? (
          <div className="mt-3 border-t border-border/60">
            {[...projection.steps.values()].map((step) => {
              const stepId = text(step["stepId"]);
              if (!stepId) return null;
              return (
                <div
                  key={stepId}
                  className="flex min-h-10 items-center gap-3 border-b border-border/50 py-2 last:border-b-0"
                >
                  <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                    {text(step["description"]) || stepId}
                  </span>
                  <span className="text-[11px] text-text-tertiary">
                    {text(step["status"]) || "unknown"}
                  </span>
                  {actorCan(projection, "retry", stepId) ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      aria-label="Retry step"
                      disabled={busy}
                      onClick={() => void onRetry(stepId)}
                    >
                      <RotateCcw />
                    </Button>
                  ) : null}
                  {actorCan(projection, "skip", stepId) ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      aria-label="Skip step"
                      disabled={busy}
                      onClick={() => void onSkip(stepId)}
                    >
                      <SkipForward />
                    </Button>
                  ) : null}
                </div>
              );
            })}
          </div>
        ) : null}

        {taskStatus === "active" ? (
          <div className="mt-3 flex gap-2 border-t border-border/60 pt-3">
            <Input
              value={steering}
              maxLength={32_768}
              disabled={busy}
              aria-label="Steer active task"
              onChange={(event) => setSteering(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void submitSteering();
              }}
            />
            <Button
              type="button"
              variant="outline"
              size="icon"
              aria-label="Send steering instruction"
              disabled={busy || !steering.trim()}
              onClick={() => void submitSteering()}
            >
              <Send />
            </Button>
            {actorCan(projection, "stop") ? (
              <Button
                type="button"
                variant="outline"
                size="icon"
                aria-label="Stop active task"
                disabled={busy}
                onClick={() => void onStop()}
              >
                <CircleStop />
              </Button>
            ) : null}
          </div>
        ) : null}

        {projection.actions.size > 0 ? (
          <div className="mt-3 border-t border-border/60 pt-2 text-[11px] text-text-secondary">
            {[...projection.actions.values()].map((action) => (
              <p key={text(action["actionRequestId"])}>
                {text(action["action"]) || "Pending action"}
                {action["executable"] === false ? " - unavailable after reload" : ""}
              </p>
            ))}
          </div>
        ) : null}

        {projection.conflicts.length > 0 ? (
          <p className="mt-2 text-[11px] text-destructive">
            Actor state conflict: {projection.conflicts.at(-1)?.code}
          </p>
        ) : null}
      </div>
    </section>
  );
}
