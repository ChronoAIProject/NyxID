import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  applyCurrentStateResult,
  createChatActorProjection,
  type ChatActorProjection,
} from "@/lib/assistant/chat-actor-state";
import {
  sendChatCommand,
  type ChatCommand,
} from "@/lib/assistant/chat-api";
import {
  chatHistoryApi,
  ChatHistoryApiError,
} from "@/lib/assistant/chat-history-api";
import {
  runChatStream,
  STREAM_PROGRESS_TIMEOUT_MS,
  type ChatEntryPatch,
} from "@/lib/assistant/chat-stream-orchestrator";
import {
  createClientId,
  createDraftChatSession,
  hydrateStoredMessages,
  resolveStoredConversationStatus,
} from "@/lib/assistant/chat-session-state";
import {
  chatErrorMessage,
  ChatProgressTimeoutError,
  currentActorTurnId,
  ReaderStoppedError,
} from "@/lib/assistant/chat-session-runtime";
import type {
  ChatConversationDetail,
  ChatSessionState,
  ConversationMeta,
} from "@/lib/assistant/chat-types";
import { isLegacyConversationId } from "@/lib/assistant/aevatar-transport";
import { useAssistantChatControls } from "@/hooks/use-assistant-chat-controls";

export const ACTIVE_STATE_REFRESH_DELAYS_MS = [250, 500, 1_000, 2_000] as const;
export { ChatProgressTimeoutError, STREAM_PROGRESS_TIMEOUT_MS };

export type ChatDetailState =
  | { readonly status: "idle" }
  | { readonly status: "loading" }
  | { readonly status: "missing" }
  | { readonly status: "error"; readonly message: string };

interface ChatEntry {
  readonly session: ChatSessionState;
  readonly projection: ChatActorProjection | null;
  readonly detailState: ChatDetailState;
  readonly actionOverrides: ReadonlyMap<
    string,
    { readonly status?: string; readonly note?: string }
  >;
}

type ControlCommand = Exclude<ChatCommand, { readonly type: "text" }>;

interface UseAssistantChatOptions {
  readonly selectedConversationId?: string;
  readonly onConversationAdopted?: (conversationId: string) => void;
  readonly onConversationMissing?: (conversationId: string) => void;
}

function createEntry(
  session: ChatSessionState,
  projection: ChatActorProjection | null = null,
  detailState: ChatDetailState = { status: "idle" },
): ChatEntry {
  return {
    session,
    projection,
    detailState,
    actionOverrides: new Map(),
  };
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function loadTranscriptWithPendingRetry(
  conversationId: string,
): Promise<{ detail: ChatConversationDetail; pendingExhausted: boolean }> {
  let detail = await chatHistoryApi.loadConversation(conversationId);
  for (const retryDelay of ACTIVE_STATE_REFRESH_DELAYS_MS) {
    if (detail.projectionStatus !== "pending" || detail.messages.length > 0) {
      return { detail, pendingExhausted: false };
    }
    await delay(retryDelay);
    detail = await chatHistoryApi.loadConversation(conversationId);
  }
  return {
    detail,
    pendingExhausted:
      detail.projectionStatus === "pending" && detail.messages.length === 0,
  };
}

export function useAssistantChat({
  selectedConversationId,
  onConversationAdopted,
  onConversationMissing,
}: UseAssistantChatOptions) {
  const [conversations, setConversations] = useState<ConversationMeta[]>([]);
  const [listLoading, setListLoading] = useState(true);
  const [listError, setListError] = useState<string>();
  const [entries, setEntries] = useState<ReadonlyMap<string, ChatEntry>>(
    () => new Map(),
  );
  const [selectedKey, setSelectedKey] = useState<string>();
  const [controlBusyKey, setControlBusyKey] = useState<string>();
  const entriesRef = useRef(entries);
  const selectedKeyRef = useRef(selectedKey);
  const routeSelectionRef = useRef<string | null | undefined>(undefined);
  const detailRequestsRef = useRef(new Map<string, string>());
  const streamControllersRef = useRef(new Map<string, AbortController>());
  const activeRefreshesRef = useRef(new Map<string, AbortController>());
  const controlControllerRef = useRef<AbortController | null>(null);

  entriesRef.current = entries;
  selectedKeyRef.current = selectedKey;

  const commitEntries = useCallback((next: ReadonlyMap<string, ChatEntry>) => {
    entriesRef.current = next;
    setEntries(next);
  }, []);

  const putEntry = useCallback(
    (key: string, entry: ChatEntry) => {
      const next = new Map(entriesRef.current);
      next.set(key, entry);
      commitEntries(next);
    },
    [commitEntries],
  );

  const updateEntry = useCallback(
    (key: string, update: Partial<ChatEntry>) => {
      const entry = entriesRef.current.get(key);
      if (!entry) return;
      const next = new Map(entriesRef.current);
      next.set(key, { ...entry, ...update });
      commitEntries(next);
    },
    [commitEntries],
  );

  const updateStreamEntry = useCallback(
    (key: string, patch: ChatEntryPatch) => {
      const entry = entriesRef.current.get(key);
      if (!entry) return;
      updateEntry(key, {
        ...(patch.session ? { session: patch.session } : {}),
        ...(Object.hasOwn(patch, "projection")
          ? { projection: patch.projection }
          : {}),
      });
    },
    [updateEntry],
  );

  const selectKey = useCallback((key: string) => {
    selectedKeyRef.current = key;
    setSelectedKey(key);
  }, []);

  const createNewChat = useCallback(() => {
    const session = createDraftChatSession();
    routeSelectionRef.current = null;
    putEntry(session.clientId, createEntry(session));
    selectKey(session.clientId);
    return session.clientId;
  }, [putEntry, selectKey]);

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
      entryKey = conversationId,
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
      const entry = entriesRef.current.get(entryKey);
      if (entry?.session.conversationId === conversationId) {
        updateEntry(entryKey, { projection: result.projection });
      }
      return result.projection;
    },
    [updateEntry],
  );

  const restoreConversation = useCallback(
    async (conversationId: string) => {
      const existing = entriesRef.current.get(conversationId);
      if (existing) return;
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
      detailRequestsRef.current.set(conversationId, requestId);
      putEntry(
        conversationId,
        createEntry(
          placeholder,
          createChatActorProjection(conversationId),
          { status: "loading" },
        ),
      );
      const [detailResult, stateResult] = await Promise.allSettled([
        loadTranscriptWithPendingRetry(conversationId),
        loadActorState(
          conversationId,
          createChatActorProjection(conversationId),
          undefined,
          false,
          conversationId,
        ),
      ]);
      if (detailRequestsRef.current.get(conversationId) !== requestId) return;
      const current = entriesRef.current.get(conversationId);
      if (!current || current.session.status === "streaming") return;
      const projection =
        stateResult.status === "fulfilled"
          ? stateResult.value
          : current.projection ?? createChatActorProjection(conversationId);
      if (detailResult.status === "fulfilled") {
        const { detail, pendingExhausted } = detailResult.value;
        const restored: ChatSessionState = {
          ...placeholder,
          latestTurnId: currentActorTurnId(projection) || undefined,
          messages: hydrateStoredMessages(detail.messages),
          status: resolveStoredConversationStatus(detail.messages),
          runtime: {
            actorId: projection.actorId ?? undefined,
            runId: currentActorTurnId(projection) || undefined,
          },
        };
        updateEntry(conversationId, {
          detailState: pendingExhausted
            ? { status: "missing" }
            : { status: "idle" },
          projection,
          session: restored,
        });
        return;
      }
      const error = detailResult.reason;
      if (error instanceof ChatHistoryApiError && error.status === 404) {
        if (!meta && !listError) {
          const next = new Map(entriesRef.current);
          next.delete(conversationId);
          commitEntries(next);
          if (selectedKeyRef.current === conversationId) {
            createNewChat();
            onConversationMissing?.(conversationId);
          }
          return;
        }
        updateEntry(conversationId, {
          detailState: { status: "missing" },
          projection,
        });
        return;
      }
      updateEntry(conversationId, {
        detailState: { status: "error", message: chatErrorMessage(error) },
        projection,
      });
    },
    [
      commitEntries,
      conversations,
      createNewChat,
      listError,
      loadActorState,
      onConversationMissing,
      putEntry,
      updateEntry,
    ],
  );

  useEffect(() => {
    const controller = new AbortController();
    void refreshConversations(controller.signal);
    return () => controller.abort();
  }, [refreshConversations]);

  useEffect(() => {
    if (listLoading) return;
    const routeKey = selectedConversationId ?? null;
    if (routeSelectionRef.current === routeKey) return;
    routeSelectionRef.current = routeKey;
    if (!selectedConversationId) {
      createNewChat();
      return;
    }
    selectKey(selectedConversationId);
    void restoreConversation(selectedConversationId);
  }, [
    createNewChat,
    listLoading,
    restoreConversation,
    selectKey,
    selectedConversationId,
  ]);

  useEffect(() => {
    const candidates = new Set<string>();
    for (const [key, entry] of entries) {
      if (
        entry.session.status === "streaming" &&
        entry.session.conversationId &&
        !isLegacyConversationId(entry.session.conversationId) &&
        (entry.projection?.stateVersion ?? 0) === 0
      ) {
        candidates.add(key);
      }
    }
    for (const [key, controller] of activeRefreshesRef.current) {
      if (!candidates.has(key)) {
        controller.abort();
        activeRefreshesRef.current.delete(key);
      }
    }
    for (const key of candidates) {
      if (activeRefreshesRef.current.has(key)) continue;
      const controller = new AbortController();
      activeRefreshesRef.current.set(key, controller);
      void (async () => {
        try {
          for (const refreshDelay of ACTIVE_STATE_REFRESH_DELAYS_MS) {
            await delay(refreshDelay);
            if (controller.signal.aborted) return;
            const entry = entriesRef.current.get(key);
            const conversationId = entry?.session.conversationId;
            const projection = entry?.projection;
            if (!entry || !conversationId || !projection) return;
            if (projection.stateVersion > 0 || entry.session.status !== "streaming") {
              return;
            }
            try {
              await loadActorState(
                conversationId,
                projection,
                controller.signal,
                false,
                key,
              );
            } catch {
              // The bounded window leaves unavailable controls version-fenced.
            }
          }
        } finally {
          if (activeRefreshesRef.current.get(key) === controller) {
            activeRefreshesRef.current.delete(key);
          }
        }
      })();
    }
  }, [entries, loadActorState]);

  const adoptEntry = useCallback(
    (
      previousKey: string,
      conversationId: string,
      session: ChatSessionState,
      projection: ChatActorProjection,
      controller: AbortController,
    ): string => {
      const previous = entriesRef.current.get(previousKey);
      if (!previous) return previousKey;
      const next = new Map(entriesRef.current);
      next.delete(previousKey);
      next.set(conversationId, { ...previous, session, projection });
      commitEntries(next);
      if (streamControllersRef.current.get(previousKey) === controller) {
        streamControllersRef.current.delete(previousKey);
        streamControllersRef.current.set(conversationId, controller);
      }
      if (previousKey !== conversationId && selectedKeyRef.current === previousKey) {
        selectKey(conversationId);
        onConversationAdopted?.(conversationId);
      }
      return conversationId;
    },
    [commitEntries, onConversationAdopted, selectKey],
  );

  const streamCommand = useCallback(
    async (
      entryKey: string,
      base: ChatSessionState,
      command: ChatCommand,
      safeUserText: string,
    ) => {
      if (streamControllersRef.current.has(entryKey)) return;
      const entry = entriesRef.current.get(entryKey);
      if (!entry || entry.session.status === "streaming") return;
      const controller = new AbortController();
      streamControllersRef.current.set(entryKey, controller);
      detailRequestsRef.current.set(entryKey, createClientId());
      updateEntry(entryKey, { detailState: { status: "idle" } });
      const initialProjection =
        entry.projection?.actorId === (base.conversationId ?? null)
          ? entry.projection
          : createChatActorProjection(base.conversationId ?? null);
      try {
        return await runChatStream({
          adoptEntry,
          base,
          command,
          controller,
          entryKey,
          initialProjection,
          loadActorState: async (
            conversationId,
            current,
            signal,
            useCursor,
            targetKey,
          ) =>
            loadActorState(
              conversationId,
              current,
              signal,
              useCursor,
              targetKey,
            ),
          refreshConversations: () => refreshConversations(),
          safeUserText,
          updateEntry: updateStreamEntry,
        });
      } finally {
        for (const [key, activeController] of streamControllersRef.current) {
          if (activeController === controller) {
            streamControllersRef.current.delete(key);
          }
        }
      }
    },
    [
      adoptEntry,
      loadActorState,
      refreshConversations,
      updateEntry,
      updateStreamEntry,
    ],
  );

  const send = useCallback(
    async (content: string) => {
      const value = content.trim();
      const key = selectedKeyRef.current;
      const entry = key ? entriesRef.current.get(key) : undefined;
      if (
        !value ||
        !key ||
        !entry ||
        entry.session.status === "streaming" ||
        (entry.session.conversationId &&
          isLegacyConversationId(entry.session.conversationId))
      ) {
        return;
      }
      await streamCommand(
        key,
        entry.session,
        {
          type: "text",
          ...(entry.session.conversationId
            ? { conversationId: entry.session.conversationId }
            : {}),
          clientRequestId: createClientId(),
          prompt: value,
        },
        value,
      );
    },
    [streamCommand],
  );

  const controlContext = useCallback(() => {
    const key = selectedKeyRef.current;
    const entry = key ? entriesRef.current.get(key) : undefined;
    const conversationId = entry?.session.conversationId;
    const state = entry?.projection;
    if (
      !key ||
      !conversationId ||
      isLegacyConversationId(conversationId) ||
      !state ||
      state.stateVersion <= 0
    ) {
      return null;
    }
    return { conversationId, key, session: entry.session, state };
  }, []);

  const dispatchAcceptedCommand = useCallback(
    async (key: string, command: ControlCommand) => {
      if (controlControllerRef.current) return;
      const controller = new AbortController();
      controlControllerRef.current = controller;
      setControlBusyKey(key);
      try {
        await sendChatCommand(command, controller.signal);
        const projection = entriesRef.current.get(key)?.projection ?? null;
        try {
          await loadActorState(
            command.conversationId,
            projection,
            controller.signal,
            true,
            key,
          );
        } catch {
          // A 202 receipt is dispatch-only; stale actor state remains honest.
        }
      } finally {
        if (controlControllerRef.current === controller) {
          controlControllerRef.current = null;
          setControlBusyKey(undefined);
        }
      }
    },
    [loadActorState],
  );

  const abortStream = useCallback((key: string, reason: ReaderStoppedError) => {
    streamControllersRef.current.get(key)?.abort(reason);
  }, []);
  const currentSelectedKey = useCallback(() => selectedKeyRef.current, []);
  const {
    controlStep,
    reportAction,
    resolveApproval,
    resolveInput,
    resolvePlan,
    steer,
    stop,
  } = useAssistantChatControls({
    abortStream,
    controlContext,
    dispatchAcceptedCommand,
    selectedKey: currentSelectedKey,
    streamCommand,
  });

  const deleteConversation = useCallback(
    async (conversationId: string) => {
      if (streamControllersRef.current.has(conversationId)) return;
      await chatHistoryApi.deleteConversation(conversationId);
      setConversations((current) =>
        current.filter((item) => item.id !== conversationId),
      );
      const next = new Map(entriesRef.current);
      next.delete(conversationId);
      commitEntries(next);
      if (selectedKeyRef.current === conversationId) {
        createNewChat();
        // Keep the route value acknowledged until the page's post-delete
        // navigation lands, or this deleted id is immediately restored.
        routeSelectionRef.current = conversationId;
      }
    },
    [commitEntries, createNewChat],
  );

  const setActionOverride = useCallback(
    (actionRequestId: string, status?: string, note?: string) => {
      const key = selectedKeyRef.current;
      const entry = key ? entriesRef.current.get(key) : undefined;
      if (!key || !entry) return;
      const overrides = new Map(entry.actionOverrides);
      if (!status && !note) overrides.delete(actionRequestId);
      else overrides.set(actionRequestId, { status, note });
      updateEntry(key, { actionOverrides: overrides });
    },
    [updateEntry],
  );

  useEffect(
    () => () => {
      for (const controller of streamControllersRef.current.values()) {
        controller.abort(new ReaderStoppedError());
      }
      for (const controller of activeRefreshesRef.current.values()) {
        controller.abort();
      }
      controlControllerRef.current?.abort();
    },
    [],
  );

  const selectedEntry = selectedKey ? entries.get(selectedKey) : undefined;
  const session = selectedEntry?.session ?? null;
  const projection = selectedEntry?.projection ?? null;
  const visibleConversations = useMemo(() => {
    const visible = [...conversations];
    const known = new Set(visible.map((item) => item.id));
    for (const entry of entries.values()) {
      const current = entry.session;
      if (!current.conversationId || known.has(current.conversationId)) continue;
      const timestamps = current.messages.map((message) => message.timestamp);
      visible.push({
        createdAt: new Date(timestamps[0] ?? Date.now()).toISOString(),
        id: current.conversationId,
        messageCount: current.expectedTurnCount,
        title: current.title,
        updatedAt: new Date(timestamps.at(-1) ?? Date.now()).toISOString(),
      });
      known.add(current.conversationId);
    }
    return visible;
  }, [conversations, entries]);

  return {
    actionOverrides: selectedEntry?.actionOverrides ?? new Map(),
    controlBusy: controlBusyKey === selectedKey,
    controlReady: Boolean(projection && projection.stateVersion > 0),
    controlStep,
    deleteConversation,
    detailState: selectedEntry?.detailState ?? { status: "idle" as const },
    isConversationStreaming: (conversationId: string) =>
      entriesRef.current.get(conversationId)?.session.status === "streaming",
    isStreaming: session?.status === "streaming",
    listError,
    listLoading,
    newChat: createNewChat,
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
