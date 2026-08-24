import { AGUIEventType } from "@/lib/assistant/agui-types";
import {
  decodeActorFrame,
  reduceActorFrame,
  type ChatActorProjection,
} from "@/lib/assistant/chat-actor-state";
import {
  extractChatStreamArtifacts,
  readChatStreamFrames,
  sendChatCommand,
  type ChatCommand,
} from "@/lib/assistant/chat-api";
import {
  buildAssistantMessagePatch,
  createChatMessage,
  createClientId,
  trimChatTitle,
} from "@/lib/assistant/chat-session-state";
import {
  chatErrorMessage,
  ChatProgressTimeoutError,
  ChatStartTimeoutError,
  isKeepaliveEvent,
  isRunStoppedEvent,
  ReaderStoppedError,
  updateSessionMessage,
} from "@/lib/assistant/chat-session-runtime";
import type { ChatMessage, ChatSessionState } from "@/lib/assistant/chat-types";
import {
  applyRuntimeEvent,
  createRuntimeEventAccumulator,
  isRawObserved,
} from "@/lib/assistant/runtime-event-semantics";

export const STREAM_PROGRESS_TIMEOUT_MS = 120_000;
export const STREAM_START_DEADLINE_MS = 30_000;

export interface ChatEntryPatch {
  readonly projection?: ChatActorProjection | null;
  readonly session?: ChatSessionState;
}

export interface ChatStreamOrchestratorOptions {
  readonly base: ChatSessionState;
  readonly command: ChatCommand;
  readonly controller: AbortController;
  readonly entryKey: string;
  readonly initialProjection: ChatActorProjection;
  readonly safeUserText: string;
  readonly adoptEntry: (
    previousKey: string,
    conversationId: string,
    session: ChatSessionState,
    projection: ChatActorProjection,
    controller: AbortController,
  ) => string;
  readonly loadActorState: (
    conversationId: string,
    current: ChatActorProjection,
    signal: AbortSignal,
    useCursor: boolean,
    entryKey: string,
  ) => Promise<ChatActorProjection>;
  readonly refreshConversations: () => Promise<void>;
  readonly updateEntry: (entryKey: string, patch: ChatEntryPatch) => void;
}

export async function runChatStream({
  base,
  command,
  controller,
  entryKey,
  initialProjection,
  safeUserText,
  adoptEntry,
  loadActorState,
  refreshConversations,
  updateEntry,
}: ChatStreamOrchestratorOptions): Promise<string | undefined> {
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
  let activeKey = entryKey;
  let actorState = initialProjection;
  const accumulator = createRuntimeEventAccumulator();
  const rawFrames: unknown[] = [];
  let authoritativeConversationId = "";
  let authoritativeTurnId = "";
  let adopted = false;
  let serverStopped = false;
  let progressWatchdog: ReturnType<typeof setTimeout> | undefined;
  let streamStarted = false;

  updateEntry(activeKey, { projection: actorState, session: streaming });

  const bestEffortStop = () => {
    if (
      !authoritativeConversationId ||
      !authoritativeTurnId ||
      actorState.stateVersion <= 0
    ) {
      return;
    }
    void sendChatCommand(
      {
        type: "task.stop",
        conversationId: authoritativeConversationId,
        turnId: authoritativeTurnId,
        stopRequestId: createClientId(),
        clientRequestId: createClientId(),
        expectedStateVersion: actorState.stateVersion,
      },
      new AbortController().signal,
    ).catch(() => undefined);
  };
  const armProgressWatchdog = () => {
    if (progressWatchdog) clearTimeout(progressWatchdog);
    progressWatchdog = setTimeout(() => {
      bestEffortStop();
      controller.abort(new ChatProgressTimeoutError());
    }, STREAM_PROGRESS_TIMEOUT_MS);
  };
  const observeMeaningfulFrame = () => {
    if (!streamStarted) {
      streamStarted = true;
      clearTimeout(startDeadline);
    }
    armProgressWatchdog();
  };
  const startDeadline = setTimeout(() => {
    controller.abort(new ChatStartTimeoutError());
  }, STREAM_START_DEADLINE_MS);

  try {
    const response = await sendChatCommand(command, controller.signal);
    for await (const frame of readChatStreamFrames(response, {
      signal: controller.signal,
    })) {
      rawFrames.push(frame.raw);
      const actorFrame = decodeActorFrame(frame.raw);
      if (actorFrame.type !== "ignored") {
        observeMeaningfulFrame();
        actorState = reduceActorFrame(actorState, actorFrame);
        updateEntry(activeKey, { projection: actorState });
      }
      if (!frame.event) continue;
      if (!isKeepaliveEvent(frame.event)) observeMeaningfulFrame();
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
        if (
          command.conversationId &&
          command.conversationId !== conversationId
        ) {
          throw new Error("Chat returned a different conversation identity.");
        }
        if (
          (authoritativeConversationId &&
            authoritativeConversationId !== conversationId) ||
          (authoritativeTurnId && authoritativeTurnId !== turnId)
        ) {
          throw new Error(
            "Chat RUN_STARTED identity changed during the stream.",
          );
        }
        authoritativeConversationId = conversationId;
        authoritativeTurnId = turnId;
        if (actorState.actorId && actorState.actorId !== conversationId) {
          throw new Error("Actor state does not match the chat conversation.");
        }
        adopted = true;
        actorState = { ...actorState, actorId: conversationId };
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
        activeKey = adoptEntry(
          activeKey,
          conversationId,
          streaming,
          actorState,
          controller,
        );
        try {
          actorState = await loadActorState(
            conversationId,
            actorState,
            controller.signal,
            false,
            activeKey,
          );
          updateEntry(activeKey, { projection: actorState });
        } catch {
          // Live facts stay visible while controls remain version-fenced.
        }
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
      updateEntry(activeKey, { session: streaming });
    }
    if (controller.signal.aborted) throw controller.signal.reason;
    if (!authoritativeConversationId || !authoritativeTurnId) {
      throw new Error(
        "Chat stream ended without authoritative conversation and turn identities.",
      );
    }
    const artifacts = extractChatStreamArtifacts(rawFrames);
    const final = updateSessionMessage(
      {
        ...streaming,
        status: serverStopped
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
        serverStopped
          ? "complete"
          : accumulator.errorText
            ? "error"
            : "complete",
      ),
    );
    updateEntry(activeKey, { projection: actorState, session: final });
    await refreshConversations();
    try {
      actorState = await loadActorState(
        authoritativeConversationId,
        actorState,
        controller.signal,
        false,
        activeKey,
      );
      updateEntry(activeKey, { projection: actorState });
    } catch {
      // Terminal materialization is eventually consistent.
    }
    return authoritativeConversationId;
  } catch (error) {
    const reason = controller.signal.aborted ? controller.signal.reason : error;
    const stopped = reason instanceof ReaderStoppedError || serverStopped;
    if (!adopted && reason instanceof ChatStartTimeoutError) {
      accumulator.errorText = reason.message;
      const failed = updateSessionMessage(
        { ...streaming, status: "error" },
        assistantMessageId,
        buildAssistantMessagePatch(accumulator, "error"),
      );
      updateEntry(entryKey, { projection: actorState, session: failed });
      return undefined;
    }
    if (!adopted && !stopped) {
      updateEntry(entryKey, { projection: initialProjection, session: base });
      throw reason;
    }
    if (!stopped) accumulator.errorText = chatErrorMessage(reason);
    const failed = updateSessionMessage(
      { ...streaming, status: stopped ? "stopped" : "error" },
      assistantMessageId,
      buildAssistantMessagePatch(accumulator, stopped ? "complete" : "error"),
    );
    updateEntry(activeKey, { projection: actorState, session: failed });
    if (adopted) await refreshConversations().catch(() => undefined);
    return authoritativeConversationId || undefined;
  } finally {
    clearTimeout(startDeadline);
    if (progressWatchdog) clearTimeout(progressWatchdog);
  }
}
