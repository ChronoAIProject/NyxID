import { useCallback } from "react";
import {
  actorCan,
  type ChatActorProjection,
  type ChatActorStep,
  type ChatPendingInput,
} from "@/lib/assistant/chat-actor-state";
import type {
  ChatCommand,
  ChatInputAnswer,
} from "@/lib/assistant/chat-api";
import { createClientId, stringField } from "@/lib/assistant/chat-session-state";
import { currentActorTurnId, ReaderStoppedError } from "@/lib/assistant/chat-session-runtime";
import type { ChatSessionState } from "@/lib/assistant/chat-types";
import type { ChatPlanGate } from "@/lib/assistant/chat-task-plan";
import type { ActionReport } from "@/schemas/assistant-actions";

type ControlCommand = Exclude<ChatCommand, { readonly type: "text" }>;

export interface AssistantControlContext {
  readonly conversationId: string;
  readonly key: string;
  readonly session: ChatSessionState;
  readonly state: ChatActorProjection;
}

interface UseAssistantChatControlsOptions {
  readonly abortStream: (key: string, reason: ReaderStoppedError) => void;
  readonly controlContext: () => AssistantControlContext | null;
  readonly dispatchAcceptedCommand: (
    key: string,
    command: ControlCommand,
  ) => Promise<void>;
  readonly selectedKey: () => string | undefined;
  readonly streamCommand: (
    entryKey: string,
    base: ChatSessionState,
    command: ChatCommand,
    safeUserText: string,
  ) => Promise<string | undefined>;
}

export function useAssistantChatControls({
  abortStream,
  controlContext,
  dispatchAcceptedCommand,
  selectedKey,
  streamCommand,
}: UseAssistantChatControlsOptions) {
  const resolveInput = useCallback(
    async (answer: ChatInputAnswer, input: ChatPendingInput) => {
      const context = controlContext();
      if (context?.state.pendingInput?.requestId !== input.requestId) return;
      await dispatchAcceptedCommand(context.key, {
        type: "input.resolve",
        conversationId: context.conversationId,
        requestId: input.requestId,
        clientRequestId: createClientId(),
        answer,
        expectedStateVersion: context.state.stateVersion,
      });
    },
    [controlContext, dispatchAcceptedCommand],
  );

  const resolveApproval = useCallback(
    async (requestId: string, approved: boolean, reason?: string) => {
      const context = controlContext();
      const pendingId =
        context?.state.pendingApproval?.approvalRequestId ??
        context?.state.pendingApproval?.requestId;
      if (!context || pendingId !== requestId) return;
      await dispatchAcceptedCommand(context.key, {
        type: "approval.resolve",
        conversationId: context.conversationId,
        requestId,
        clientRequestId: createClientId(),
        approved,
        ...(reason?.trim() ? { reason: reason.trim() } : {}),
        expectedStateVersion: context.state.stateVersion,
      });
    },
    [controlContext, dispatchAcceptedCommand],
  );

  const resolvePlan = useCallback(
    async (confirmed: boolean, gate: ChatPlanGate) => {
      const context = controlContext();
      if (
        !context ||
        gate.mode !== "confirm" ||
        gate.status !== "pending" ||
        !gate.requestId ||
        !gate.taskId ||
        !gate.planId ||
        gate.planRevision === undefined
      ) return;
      await dispatchAcceptedCommand(context.key, {
        type: "plan.resolve",
        conversationId: context.conversationId,
        taskId: gate.taskId,
        planId: gate.planId,
        requestId: gate.requestId,
        clientRequestId: createClientId(),
        planRevision: gate.planRevision,
        confirmed,
        expectedStateVersion: context.state.stateVersion,
      });
    },
    [controlContext, dispatchAcceptedCommand],
  );

  const stop = useCallback(async () => {
    const key = selectedKey();
    const context = controlContext();
    const turnId = currentActorTurnId(context?.state ?? null);
    if (context && turnId && actorCan(context.state, "stop")) {
      await dispatchAcceptedCommand(context.key, {
        type: "task.stop",
        conversationId: context.conversationId,
        turnId,
        stopRequestId: createClientId(),
        clientRequestId: createClientId(),
        expectedStateVersion: context.state.stateVersion,
      });
    }
    if (key) abortStream(key, new ReaderStoppedError());
  }, [abortStream, controlContext, dispatchAcceptedCommand, selectedKey]);

  const steer = useCallback(
    async (instruction: string) => {
      const context = controlContext();
      const turnId = currentActorTurnId(context?.state ?? null);
      if (!context || !turnId || !instruction.trim()) return;
      await dispatchAcceptedCommand(context.key, {
        type: "task.steer",
        conversationId: context.conversationId,
        turnId,
        steeringId: createClientId(),
        clientRequestId: createClientId(),
        instruction,
        expectedStateVersion: context.state.stateVersion,
      });
    },
    [controlContext, dispatchAcceptedCommand],
  );

  const controlStep = useCallback(
    async (type: "step.retry" | "step.skip", step: ChatActorStep) => {
      const context = controlContext();
      const turnId =
        stringField(step.operation, "turnId") ||
        currentActorTurnId(context?.state ?? null);
      const taskId =
        stringField(step.operation, "taskId") || context?.state.task?.taskId || "";
      const generation = step.operation?.operationGeneration;
      if (
        !context ||
        !turnId ||
        !taskId ||
        typeof generation !== "number" ||
        !actorCan(
          context.state,
          type === "step.retry" ? "retry" : "skip",
          step.stepId,
        )
      ) return;
      const requestId = createClientId();
      await dispatchAcceptedCommand(context.key, {
        type,
        conversationId: context.conversationId,
        turnId,
        taskId,
        stepId: step.stepId,
        ...(type === "step.retry"
          ? { retryRequestId: requestId }
          : { skipRequestId: requestId }),
        clientRequestId: createClientId(),
        expectedOperationGeneration: generation,
        expectedStateVersion: context.state.stateVersion,
      });
    },
    [controlContext, dispatchAcceptedCommand],
  );

  const reportAction = useCallback(
    async (report: ActionReport) => {
      const context = controlContext();
      if (!context) return;
      await streamCommand(
        context.key,
        context.session,
        {
          type: "action.continue",
          conversationId: context.conversationId,
          originTurnId: report.originTurnId,
          clientRequestId: createClientId(),
          actions: [report],
        },
        `NyxID action update: ${report.disposition}.`,
      );
    },
    [controlContext, streamCommand],
  );

  return {
    controlStep,
    reportAction,
    resolveApproval,
    resolveInput,
    resolvePlan,
    steer,
    stop,
  } as const;
}
