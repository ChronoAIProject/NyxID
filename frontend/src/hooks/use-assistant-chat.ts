import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { AGUIEventType } from "@/lib/assistant/agui-types";
import {
  actorCan,
  applyCurrentStateResult,
  createChatActorProjection,
  decodeActorFrame,
  reduceActorFrame,
  type ChatActorProjection,
  type ChatActorStep,
  type ChatPendingInput,
} from "@/lib/assistant/chat-actor-state";
import {
  extractChatStreamArtifacts,
  readChatStreamFrames,
  sendChatCommand,
  type ChatCommand,
  type ChatInputAnswer,
} from "@/lib/assistant/chat-api";
import { chatHistoryApi } from "@/lib/assistant/chat-history-api";
import {
  buildAssistantMessagePatch,
  createChatMessage,
  createClientId,
  createDraftChatSession,
  hydrateStoredMessages,
  resolveStoredConversationStatus,
  stringField,
  trimChatTitle,
} from "@/lib/assistant/chat-session-state";
import {
  chatErrorMessage,
  ChatProgressTimeoutError,
  currentActorTurnId,
  isKeepaliveEvent,
  isRunStoppedEvent,
  ReaderStoppedError,
  updateSessionMessage,
} from "@/lib/assistant/chat-session-runtime";
import type {
  ChatMessage,
  ChatSessionState,
  ConversationMeta,
} from "@/lib/assistant/chat-types";
import {
  applyRuntimeEvent,
  createRuntimeEventAccumulator,
  isRawObserved,
} from "@/lib/assistant/runtime-event-semantics";
import type { ChatPlanGate } from "@/lib/assistant/chat-task-plan";
import type { ActionReport } from "@/schemas/assistant-actions";

export const ACTIVE_STATE_REFRESH_DELAYS_MS = [250, 500, 1_000, 2_000] as const;
export const STREAM_PROGRESS_TIMEOUT_MS = 120_000;
export { ChatProgressTimeoutError };

type DetailState =
  | { readonly status: "idle" }
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly message: string };

type ControlCommand = Exclude<ChatCommand, { readonly type: "text" }>;
interface UseAssistantChatOptions {
  readonly selectedConversationId?: string;
  readonly onConversationAdopted?: (conversationId: string) => void;
}

export function useAssistantChat({
  selectedConversationId,
  onConversationAdopted,
}: UseAssistantChatOptions) {
  const [conversations, setConversations] = useState<ConversationMeta[]>([]);
  const [listLoading, setListLoading] = useState(true);
  const [listError, setListError] = useState<string>();
  const [session, setSession] = useState<ChatSessionState | null>(null);
  const [projection, setProjection] = useState<ChatActorProjection | null>(null);
  const [detailState, setDetailState] = useState<DetailState>({ status: "idle" });
  const [controlBusy, setControlBusy] = useState(false);
  const [actionOverrides, setActionOverrides] = useState<
    ReadonlyMap<string, { readonly status?: string; readonly note?: string }>
  >(() => new Map());
  const sessionRef = useRef(session);
  const projectionRef = useRef(projection);
  const streamControllerRef = useRef<AbortController | null>(null);
  const controlControllerRef = useRef<AbortController | null>(null);
  const detailRequestRef = useRef("");
  const selectionRef = useRef<string | undefined | null>(null);

  sessionRef.current = session;
  projectionRef.current = projection;

  const commitSession = useCallback((next: ChatSessionState | null) => {
    sessionRef.current = next;
    setSession(next);
  }, []);

  const commitProjection = useCallback((next: ChatActorProjection | null) => {
    projectionRef.current = next;
    setProjection(next);
  }, []);

  const refreshConversations = useCallback(async (signal?: AbortSignal) => {
    setListLoading(true);
    setListError(undefined);
    try {
      const next = await chatHistoryApi.listConversationMetas(signal);
      if (!signal?.aborted) setConversations(next);
    } catch (error) {
      if (!signal?.aborted) setListError(chatErrorMessage(error));
    } finally {
      if (!signal?.aborted) setListLoading(false);
    }
  }, []);

  const loadActorState = useCallback(
    async (
      conversationId: string,
      current: ChatActorProjection | null,
      signal?: AbortSignal,
      useCursor = true,
    ): Promise<ChatActorProjection> => {
      const turnId = currentActorTurnId(current);
      const cursor =
        useCursor && current && current.stateVersion > 0
          ? {
              afterStateVersion: current.stateVersion,
              ...(turnId ? { turnId } : {}),
            }
          : {};
      const envelope = await chatHistoryApi.loadConversationState(
        conversationId,
        cursor,
        signal,
      );
      if (
        current &&
        envelope &&
        typeof envelope === "object" &&
        "status" in envelope &&
        envelope.status === "not_found"
      ) {
        return current;
      }
      let result = applyCurrentStateResult(
        current ?? createChatActorProjection(conversationId),
        envelope,
      );
      if (result.reloadWithoutCursor) {
        result = applyCurrentStateResult(
          result.projection,
          await chatHistoryApi.loadConversationState(conversationId, {}, signal),
        );
      }
      if (sessionRef.current?.conversationId === conversationId) {
        commitProjection(result.projection);
      }
      return result.projection;
    },
    [commitProjection],
  );

  const restoreConversation = useCallback(
    async (conversationId: string) => {
      if (sessionRef.current?.status === "streaming") return;
      const meta = conversations.find((item) => item.id === conversationId);
      const requestId = createClientId();
      const placeholder: ChatSessionState = {
        clientId: createClientId(),
        conversationId,
        expectedTurnCount: meta?.messageCount ?? 0,
        messages: [],
        status: "completed_text",
        title: meta?.title || "New chat",
      };
      detailRequestRef.current = requestId;
      commitSession(placeholder);
      commitProjection(createChatActorProjection(conversationId));
      setActionOverrides(new Map());
      setDetailState({ status: "loading" });
      try {
        const [detail, stateEnvelope] = await Promise.all([
          chatHistoryApi.loadConversation(conversationId),
          chatHistoryApi.loadConversationState(conversationId),
        ]);
        if (detailRequestRef.current !== requestId) return;
        let state = applyCurrentStateResult(
          createChatActorProjection(conversationId),
          stateEnvelope,
        );
        if (state.reloadWithoutCursor) {
          state = applyCurrentStateResult(
            state.projection,
            await chatHistoryApi.loadConversationState(conversationId),
          );
        }
        const restored: ChatSessionState = {
          ...placeholder,
          latestTurnId: currentActorTurnId(state.projection) || undefined,
          messages: hydrateStoredMessages(detail.messages),
          status: resolveStoredConversationStatus(detail.messages),
          runtime: {
            actorId: state.projection.actorId ?? undefined,
            runId: currentActorTurnId(state.projection) || undefined,
          },
        };
        commitSession(restored);
        commitProjection(state.projection);
        setDetailState({ status: "idle" });
      } catch (error) {
        if (detailRequestRef.current === requestId) {
          setDetailState({ status: "error", message: chatErrorMessage(error) });
        }
      }
    },
    [commitProjection, commitSession, conversations],
  );

  useEffect(() => {
    const controller = new AbortController();
    void refreshConversations(controller.signal);
    return () => controller.abort();
  }, [refreshConversations]);

  useEffect(() => {
    if (selectedConversationId && listLoading) return;
    if (selectionRef.current === selectedConversationId) return;
    selectionRef.current = selectedConversationId;
    if (!selectedConversationId) {
      if (sessionRef.current?.status !== "streaming") {
        detailRequestRef.current = createClientId();
        commitSession(createDraftChatSession());
        commitProjection(null);
        setDetailState({ status: "idle" });
        setActionOverrides(new Map());
      }
      return;
    }
    if (
      sessionRef.current?.conversationId === selectedConversationId &&
      sessionRef.current.status !== "draft"
    ) {
      return;
    }
    void restoreConversation(selectedConversationId);
  }, [
    commitProjection,
    commitSession,
    listLoading,
    restoreConversation,
    selectedConversationId,
  ]);

  useEffect(() => {
    const conversationId = session?.conversationId?.trim();
    if (
      session?.status !== "streaming" ||
      !conversationId ||
      (projection?.stateVersion ?? 0) > 0
    ) {
      return;
    }
    const controller = new AbortController();
    let index = 0;
    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    const refresh = async () => {
      const current = projectionRef.current;
      if (
        controller.signal.aborted ||
        !current ||
        current.actorId !== conversationId ||
        current.stateVersion > 0
      ) {
        return;
      }
      try {
        await loadActorState(conversationId, current, controller.signal, false);
      } catch {
        // The bounded refresh window keeps version-fenced controls disabled.
      }
      if (
        !controller.signal.aborted &&
        (projectionRef.current?.stateVersion ?? 0) === 0 &&
        index < ACTIVE_STATE_REFRESH_DELAYS_MS.length
      ) {
        timeoutId = setTimeout(
          () => void refresh(),
          ACTIVE_STATE_REFRESH_DELAYS_MS[index++] ?? 0,
        );
      }
    };
    timeoutId = setTimeout(
      () => void refresh(),
      ACTIVE_STATE_REFRESH_DELAYS_MS[index++] ?? 0,
    );
    return () => {
      controller.abort();
      if (timeoutId) clearTimeout(timeoutId);
    };
  }, [loadActorState, projection?.stateVersion, session?.conversationId, session?.status]);

  const streamCommand = useCallback(
    async (
      base: ChatSessionState,
      command: ChatCommand,
      safeUserText: string,
    ): Promise<string | undefined> => {
      if (sessionRef.current?.status === "streaming") return undefined;
      const assistantMessageId = createClientId();
      const assistantMessage: ChatMessage = {
        content: "",
        events: [],
        id: assistantMessageId,
        role: "assistant",
        status: "streaming",
        steps: [],
        thinking: "",
        timestamp: Date.now(),
        toolCalls: [],
      };
      let streaming: ChatSessionState = {
        ...base,
        messages: [
          ...base.messages,
          createChatMessage("user", safeUserText),
          assistantMessage,
        ],
        status: "streaming",
        title: base.title === "New chat" ? trimChatTitle(safeUserText) : base.title,
      };
      let actorState =
        projectionRef.current?.actorId === (base.conversationId ?? null)
          ? projectionRef.current
          : createChatActorProjection(base.conversationId ?? null);
      const accumulator = createRuntimeEventAccumulator();
      const rawFrames: unknown[] = [];
      let authoritativeConversationId = "";
      let authoritativeTurnId = "";
      let serverStopped = false;
      let watchdog: ReturnType<typeof setTimeout> | undefined;
      const controller = new AbortController();
      streamControllerRef.current?.abort(new ReaderStoppedError());
      streamControllerRef.current = controller;
      detailRequestRef.current = createClientId();
      setDetailState({ status: "idle" });
      commitSession(streaming);

      const bestEffortStop = () => {
        if (!authoritativeConversationId || !authoritativeTurnId) return;
        const stateVersion = Math.max(actorState.stateVersion, 0);
        void sendChatCommand(
          {
            type: "task.stop",
            conversationId: authoritativeConversationId,
            turnId: authoritativeTurnId,
            stopRequestId: createClientId(),
            clientRequestId: createClientId(),
            expectedStateVersion: stateVersion,
          },
          new AbortController().signal,
        ).catch(() => undefined);
      };
      const armWatchdog = () => {
        if (watchdog) clearTimeout(watchdog);
        watchdog = setTimeout(() => {
          bestEffortStop();
          controller.abort(new ChatProgressTimeoutError());
        }, STREAM_PROGRESS_TIMEOUT_MS);
      };
      armWatchdog();

      try {
        const response = await sendChatCommand(command, controller.signal);
        for await (const frame of readChatStreamFrames(response, {
          signal: controller.signal,
        })) {
          rawFrames.push(frame.raw);
          const actorFrame = decodeActorFrame(frame.raw);
          if (actorFrame.type !== "ignored") {
            armWatchdog();
            actorState = reduceActorFrame(actorState, actorFrame);
            commitProjection(actorState);
          }
          if (!frame.event) continue;
          if (!isKeepaliveEvent(frame.event)) armWatchdog();
          applyRuntimeEvent(accumulator, frame.event);
          serverStopped ||= isRunStoppedEvent(frame.event);
          if (frame.event.type === AGUIEventType.RUN_STARTED) {
            const conversationId = accumulator.actorId.trim();
            const turnId = accumulator.runId.trim();
            if (!conversationId || !turnId) {
              throw new Error(
                "Chat RUN_STARTED did not contain authoritative conversation and turn identities.",
              );
            }
            if (command.conversationId && command.conversationId !== conversationId) {
              throw new Error("Chat returned a different conversation identity.");
            }
            if (
              (authoritativeConversationId &&
                authoritativeConversationId !== conversationId) ||
              (authoritativeTurnId && authoritativeTurnId !== turnId)
            ) {
              throw new Error("Chat RUN_STARTED identity changed during the stream.");
            }
            authoritativeConversationId = conversationId;
            authoritativeTurnId = turnId;
            if (actorState.actorId && actorState.actorId !== conversationId) {
              throw new Error("Actor state does not match the chat conversation.");
            }
            actorState = { ...actorState, actorId: conversationId };
            commitProjection(actorState);
            try {
              actorState = await loadActorState(
                conversationId,
                actorState,
                controller.signal,
                false,
              );
              commitProjection(actorState);
            } catch {
              // Live facts remain visible while controls remain version-fenced.
            }
            streaming = {
              ...streaming,
              conversationId,
              expectedTurnCount: base.expectedTurnCount + 1,
              latestTurnId: turnId,
              runtime: {
                actorId: conversationId,
                commandId: accumulator.commandId || undefined,
                runId: turnId,
              },
            };
            commitSession(streaming);
            onConversationAdopted?.(conversationId);
          }
          if (isRawObserved(frame.event)) continue;
          streaming = updateSessionMessage(
            {
              ...streaming,
              runtime: {
                actorId: accumulator.actorId || undefined,
                commandId: accumulator.commandId || undefined,
                runId: accumulator.runId || undefined,
              },
            },
            assistantMessageId,
            buildAssistantMessagePatch(
              accumulator,
              accumulator.errorText ? "error" : "streaming",
            ),
          );
          commitSession(streaming);
        }
        if (controller.signal.aborted) throw controller.signal.reason;
        if (!authoritativeConversationId || !authoritativeTurnId) {
          throw new Error(
            "Chat stream ended without authoritative conversation and turn identities.",
          );
        }
        const artifacts = extractChatStreamArtifacts(rawFrames);
        const stopped = serverStopped;
        const final = updateSessionMessage(
          {
            ...streaming,
            status: stopped
              ? "stopped"
              : accumulator.errorText
                ? "error"
                : "completed_text",
            target: artifacts.target ?? streaming.target,
            usage: artifacts.usage ?? streaming.usage,
          },
          assistantMessageId,
          buildAssistantMessagePatch(
            accumulator,
            stopped ? "complete" : accumulator.errorText ? "error" : "complete",
          ),
        );
        commitSession(final);
        await refreshConversations();
        try {
          actorState = await loadActorState(
            authoritativeConversationId,
            actorState,
            controller.signal,
            false,
          );
          commitProjection(actorState);
        } catch {
          // Terminal state materialization is eventually consistent.
        }
        return authoritativeConversationId;
      } catch (error) {
        const reason = controller.signal.aborted ? controller.signal.reason : error;
        const stopped = reason instanceof ReaderStoppedError || serverStopped;
        if (!stopped) accumulator.errorText = chatErrorMessage(reason);
        const failed = updateSessionMessage(
          { ...streaming, status: stopped ? "stopped" : "error" },
          assistantMessageId,
          buildAssistantMessagePatch(accumulator, stopped ? "complete" : "error"),
        );
        commitSession(failed);
        if (!stopped) throw reason;
        return authoritativeConversationId || undefined;
      } finally {
        if (watchdog) clearTimeout(watchdog);
        if (streamControllerRef.current === controller) {
          streamControllerRef.current = null;
        }
      }
    },
    [commitProjection, commitSession, loadActorState, onConversationAdopted, refreshConversations],
  );

  const send = useCallback(
    async (content: string) => {
      const value = content.trim();
      const current = sessionRef.current ?? createDraftChatSession();
      if (!value || current.status === "streaming") return;
      await streamCommand(
        current,
        {
          type: "text",
          ...(current.conversationId
            ? { conversationId: current.conversationId }
            : {}),
          clientRequestId: createClientId(),
          prompt: value,
        },
        value,
      );
    },
    [streamCommand],
  );

  const dispatchAcceptedCommand = useCallback(
    async (command: ControlCommand) => {
      if (controlBusy) return;
      const controller = new AbortController();
      controlControllerRef.current?.abort();
      controlControllerRef.current = controller;
      setControlBusy(true);
      try {
        await sendChatCommand(command, controller.signal);
        try {
          await loadActorState(
            command.conversationId,
            projectionRef.current,
            controller.signal,
          );
        } catch {
          // A 202 receipt is dispatch-only; stale state remains honest.
        }
      } finally {
        if (controlControllerRef.current === controller) {
          controlControllerRef.current = null;
        }
        setControlBusy(false);
      }
    },
    [controlBusy, loadActorState],
  );

  const controlContext = useCallback(() => {
    const conversationId = sessionRef.current?.conversationId;
    const state = projectionRef.current;
    return conversationId && state ? { conversationId, state } : null;
  }, []);

  const resolveInput = useCallback(
    async (answer: ChatInputAnswer, input: ChatPendingInput) => {
      const context = controlContext();
      if (!context || context.state.pendingInput?.requestId !== input.requestId) return;
      await dispatchAcceptedCommand({
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
      await dispatchAcceptedCommand({
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
      await dispatchAcceptedCommand({
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
    const context = controlContext();
    const turnId = currentActorTurnId(context?.state ?? null);
    if (context && turnId && actorCan(context.state, "stop")) {
      await dispatchAcceptedCommand({
        type: "task.stop",
        conversationId: context.conversationId,
        turnId,
        stopRequestId: createClientId(),
        clientRequestId: createClientId(),
        expectedStateVersion: context.state.stateVersion,
      });
    }
    streamControllerRef.current?.abort(new ReaderStoppedError());
  }, [controlContext, dispatchAcceptedCommand]);

  const steer = useCallback(
    async (instruction: string) => {
      const context = controlContext();
      const turnId = currentActorTurnId(context?.state ?? null);
      if (!context || !turnId || !instruction.trim()) return;
      await dispatchAcceptedCommand({
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
        stringField(step.operation, "turnId") || currentActorTurnId(context?.state ?? null);
      const taskId =
        stringField(step.operation, "taskId") ||
        stringField(context?.state.task as unknown as Record<string, unknown>, "taskId");
      const generation = step.operation?.operationGeneration;
      if (
        !context ||
        !turnId ||
        !taskId ||
        typeof generation !== "number" ||
        !actorCan(context.state, type === "step.retry" ? "retry" : "skip", step.stepId)
      ) return;
      const requestId = createClientId();
      await dispatchAcceptedCommand({
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
      const current = sessionRef.current;
      if (!current?.conversationId) return;
      await streamCommand(
        current,
        {
          type: "action.continue",
          conversationId: current.conversationId,
          originTurnId: report.originTurnId,
          clientRequestId: createClientId(),
          actions: [report],
        },
        `NyxID action update: ${report.disposition}.`,
      );
    },
    [streamCommand],
  );

  const deleteConversation = useCallback(
    async (conversationId: string) => {
      await chatHistoryApi.deleteConversation(conversationId);
      setConversations((current) => current.filter((item) => item.id !== conversationId));
      if (sessionRef.current?.conversationId === conversationId) {
        commitSession(createDraftChatSession());
        commitProjection(null);
      }
    },
    [commitProjection, commitSession],
  );

  const setActionOverride = useCallback(
    (actionRequestId: string, status?: string, note?: string) => {
      setActionOverrides((current) => {
        const next = new Map(current);
        if (!status && !note) next.delete(actionRequestId);
        else next.set(actionRequestId, { status, note });
        return next;
      });
    },
    [],
  );

  useEffect(
    () => () => {
      streamControllerRef.current?.abort(new ReaderStoppedError());
      controlControllerRef.current?.abort();
    },
    [],
  );

  const visibleConversations = useMemo(() => {
    const current = session;
    if (
      !current?.conversationId ||
      conversations.some((item) => item.id === current.conversationId)
    ) return conversations;
    const timestamps = current.messages.map((message) => message.timestamp);
    return [
      {
        createdAt: new Date(timestamps[0] ?? Date.now()).toISOString(),
        id: current.conversationId,
        messageCount: current.expectedTurnCount,
        title: current.title,
        updatedAt: new Date(timestamps.at(-1) ?? Date.now()).toISOString(),
      },
      ...conversations,
    ];
  }, [conversations, session]);

  return {
    actionOverrides,
    controlBusy,
    controlStep,
    deleteConversation,
    detailState,
    isStreaming: session?.status === "streaming",
    listError,
    listLoading,
    projection,
    refreshConversations,
    reportAction,
    resolveApproval,
    resolveInput,
    resolvePlan,
    send,
    session,
    setActionOverride,
    steer,
    stop,
    visibleConversations,
  } as const;
}
