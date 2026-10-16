import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import NetInfo from "@react-native-community/netinfo";
import { useQuery } from "@tanstack/react-query";
import { assistantApi, streamAssistantTurn } from "../../lib/api/assistantApi";
import { agentsApi } from "../../lib/api/agentsApi";
import { ApiError } from "../../lib/api/ApiError";
import { resolveChatAgentId } from "./assistantAgentContext";
import {
  assistantTurnBody,
  restoreRefusedAssistantSend,
} from "./assistantTurnRequest";
import type { AssistantUploadedMessage } from "./AssistantUploadSession";
import {
  acknowledgementForConversation,
  selectAssistantHistoryTarget,
} from "./assistantHistoryTarget";
import {
  applyAssistantEvent,
  emptyLiveReply,
  type AssistantLiveReply,
} from "./assistantStream";
import {
  continuationText,
  mergeAssistantHistory,
  needsClientContinuation,
} from "./assistantState";
import type {
  AssistantAcknowledgement,
  AssistantAttachment,
  AssistantConversation,
  AssistantHistory,
  AssistantModel,
} from "./assistantTypes";

export function useAssistant(
  initialId?: string,
  routeAgentId?: string,
  onConversationAdopted?: (id: string) => void,
  initialAcknowledgementId?: string,
) {
  const focused = useIsFocused();
  const [conversationId, setConversationId] = useState(initialId);
  const [conversations, setConversations] = useState<AssistantConversation[]>(
    [],
  );
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [indexedAgentId, setIndexedAgentId] = useState<string>();
  const [history, setHistory] = useState<AssistantHistory>();
  const [models, setModels] = useState<AssistantModel[]>([]);
  const [model, setModel] = useState("nyxagent/chat");
  const [live, setLive] = useState<AssistantLiveReply>();
  const [inputText, setInputText] = useState<string>();
  const [inputAttachments, setInputAttachments] = useState<
    AssistantAttachment[]
  >([]);
  const [sending, setSending] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [loadingIndex, setLoadingIndex] = useState(false);
  const [deciding, setDeciding] = useState<string>();
  const [mutating, setMutating] = useState(false);
  const [error, setError] = useState<string>();
  const [recoveryRequired, setRecoveryRequired] = useState(false);
  const [online, setOnline] = useState(true);
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const selected = useRef(initialId);
  const historyTarget = useRef(
    selectAssistantHistoryTarget(
      undefined,
      initialId,
      initialAcknowledgementId,
    ),
  );
  const latestHistory = useRef<AssistantHistory | undefined>(undefined);
  const mounted = useRef(true);
  const epoch = useRef(0);
  const historySequence = useRef(0);
  const reopenSequence = useRef(0);
  const recoverSequence = useRef(0);
  const indexSequence = useRef(0);
  const streamController = useRef<AbortController | undefined>(undefined);
  const sendLock = useRef(false);
  const sendOperation = useRef<object | undefined>(undefined);
  const decisionLock = useRef(false);
  const lastDecision = useRef(0);
  const previousAgent = useRef(routeAgentId);
  const previousAcknowledgement = useRef(initialAcknowledgementId);
  const adopt = useRef(onConversationAdopted);
  adopt.current = onConversationAdopted;
  const continuations = useRef(new Map<string, AssistantAcknowledgement[]>());
  const active = focused && foreground && online;
  const activeRef = useRef(active);
  activeRef.current = active;
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: active,
    retry: false,
    staleTime: 15_000,
  });
  const agentId = resolveChatAgentId(
    agents.data?.agents,
    history?.conversation.agent?.id,
    routeAgentId,
  );
  const agent = agents.data?.agents.find((item) => item.id === agentId);
  const indexOwner = useRef(agentId);
  indexOwner.current = agentId;
  const modelOwner = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!agent || modelOwner.current === agentId) return;
    modelOwner.current = agentId;
    setModel(agent.model ?? "nyxagent/chat");
  }, [agentId, agent]);

  const updateHistory = useCallback((value: AssistantHistory | undefined) => {
    latestHistory.current = value;
    setHistory(value);
  }, []);

  const refreshIndex = useCallback(
    async (cursor?: string) => {
      // A late refresh from the previous agent must not invalidate the new index.
      if (!agentId || indexOwner.current !== agentId) return;
      const sequence = ++indexSequence.current;
      if (mounted.current) setLoadingIndex(true);
      try {
        const page = await assistantApi.list(cursor, agentId);
        if (
          !mounted.current ||
          sequence !== indexSequence.current ||
          indexOwner.current !== agentId
        )
          return;
        setConversations((previous) => {
          const merged = new Map(
            (cursor ? previous : []).map((row) => [row.id, row]),
          );
          for (const row of page.conversations) merged.set(row.id, row);
          return [...merged.values()].sort((a, b) =>
            b.last_message_at.localeCompare(a.last_message_at),
          );
        });
        setNextCursor(page.next_cursor);
        setIndexedAgentId(agentId);
      } finally {
        if (mounted.current && sequence === indexSequence.current)
          setLoadingIndex(false);
      }
    },
    [agentId],
  );
  useEffect(() => {
    indexSequence.current += 1;
    setConversations([]);
    setNextCursor(null);
    setIndexedAgentId(undefined);
    setLoadingIndex(false);
  }, [agentId]);

  const refreshHistory = useCallback(
    async (id: string, beforeSeq?: number) => {
      const sequence = ++historySequence.current;
      const page = await assistantApi.history(
        id,
        beforeSeq,
        undefined,
        acknowledgementForConversation(historyTarget.current, id),
      );
      if (
        !mounted.current ||
        id !== selected.current ||
        sequence !== historySequence.current
      )
        return undefined;
      const merged = mergeAssistantHistory(
        latestHistory.current,
        page,
        !!beforeSeq,
      );
      updateHistory(merged);
      setRecoveryRequired(false);
      return merged;
    },
    [updateHistory],
  );

  const openConversation = useCallback(
    (id?: string, acknowledgementId?: string) => {
      const operation = ++reopenSequence.current;
      historyTarget.current = selectAssistantHistoryTarget(
        historyTarget.current,
        id,
        acknowledgementId,
      );
      if (id && id === selected.current) {
        // React does not rerun the selected-id effect for the same saved chat.
        // Keep its rows and live subscription while explicitly refreshing it.
        const localEpoch = epoch.current;
        setError(undefined);
        setLoading(true);
        void refreshHistory(id)
          .catch((cause) => {
            if (
              mounted.current &&
              selected.current === id &&
              epoch.current === localEpoch &&
              reopenSequence.current === operation
            )
              setError(errorMessage(cause));
          })
          .finally(() => {
            if (
              mounted.current &&
              selected.current === id &&
              epoch.current === localEpoch &&
              reopenSequence.current === operation
            )
              setLoading(false);
          });
        return;
      }
      epoch.current += 1;
      historySequence.current += 1;
      streamController.current?.abort();
      selected.current = id;
      setConversationId(id);
      updateHistory(undefined);
      setLive(undefined);
      setInputText(undefined);
      setInputAttachments([]);
      setError(undefined);
      setRecoveryRequired(false);
      setLoading(!!id);
      setLoadingOlder(false);
    },
    [refreshHistory, updateHistory],
  );

  useEffect(() => {
    const agentChanged = previousAgent.current !== routeAgentId;
    const acknowledgementChanged =
      previousAcknowledgement.current !== initialAcknowledgementId;
    previousAgent.current = routeAgentId;
    previousAcknowledgement.current = initialAcknowledgementId;
    if (
      agentChanged ||
      initialId !== selected.current ||
      acknowledgementChanged
    )
      openConversation(initialId, initialAcknowledgementId);
  }, [initialId, routeAgentId, initialAcknowledgementId, openConversation]);

  const recover = useCallback(async () => {
    const recovery = ++recoverSequence.current;
    setError(undefined);
    const id = selected.current;
    const localEpoch = epoch.current;
    const operation = reopenSequence.current;
    try {
      if (id) await refreshHistory(id);
      await refreshIndex();
    } catch (cause) {
      if (
        mounted.current &&
        selected.current === id &&
        epoch.current === localEpoch &&
        reopenSequence.current === operation &&
        recoverSequence.current === recovery
      )
        setError(errorMessage(cause));
    }
  }, [refreshHistory, refreshIndex]);

  useEffect(() => {
    mounted.current = true;
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    const unsubscribe = NetInfo.addEventListener((state) =>
      setOnline(
        state.isConnected !== false && state.isInternetReachable !== false,
      ),
    );
    return () => {
      mounted.current = false;
      epoch.current += 1;
      streamController.current?.abort();
      subscription.remove();
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    if (!active) {
      streamController.current?.abort();
      return;
    }
    let current = true;
    void recover();
    void assistantApi
      .models()
      .then((value) => {
        if (current && mounted.current) setModels(value);
      })
      .catch(() => undefined);
    return () => {
      current = false;
    };
  }, [active, recover]);

  useEffect(() => {
    if (!conversationId || !active) return;
    let current = true;
    const operation = ++reopenSequence.current;
    const localEpoch = epoch.current;
    setLoading(true);
    void refreshHistory(conversationId)
      .catch((cause) => {
        if (
          current &&
          mounted.current &&
          selected.current === conversationId &&
          epoch.current === localEpoch &&
          reopenSequence.current === operation
        )
          setError(errorMessage(cause));
      })
      .finally(() => {
        if (
          current &&
          mounted.current &&
          selected.current === conversationId &&
          epoch.current === localEpoch &&
          reopenSequence.current === operation
        )
          setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [conversationId, active, refreshHistory]);

  const needsPolling =
    !!history?.conversation.active_turn ||
    !!history?.acknowledgements.some((row) => row.status === "pending") ||
    !!history?.approvals.length ||
    !!history?.waiting.length ||
    sending;
  useEffect(() => {
    if (!active || !conversationId || !needsPolling || loadingOlder) return;
    let polling = false;
    const interval = setInterval(() => {
      if (polling) return;
      polling = true;
      void refreshHistory(conversationId)
        .catch(() => undefined)
        .finally(() => {
          polling = false;
        });
    }, 2000);
    return () => clearInterval(interval);
  }, [active, conversationId, needsPolling, loadingOlder, refreshHistory]);

  const send = useCallback(
    async (
      text: string,
      retryOf?: string,
      uploads?: AssistantUploadedMessage,
    ) => {
      if (sendLock.current) throw new Error("A message is already being sent.");
      if (!activeRef.current) throw new Error("Reconnect to send a message.");
      if (recoveryRequired)
        throw new Error("Refresh or open the saved chat before sending again.");
      if (
        uploads &&
        selected.current &&
        uploads.conversationId !== selected.current
      )
        throw new Error(
          "These files belong to a different chat. Choose them again here.",
        );
      if (uploads && retryOf)
        throw new Error("Retry uses the original message's attachments.");
      if (selected.current && !latestHistory.current)
        throw new Error("Wait for this chat's saved history to load.");
      if (latestHistory.current?.conversation.active_turn)
        throw new Error(
          "The assistant is still working. Stop it or wait for completion.",
        );
      if (latestHistory.current?.conversation.agent?.destroyed)
        throw new Error(
          "This agent was destroyed. Its conversation is read-only.",
        );
      if (
        !selected.current &&
        (!agent || agent.can_use !== true || agent.status === "destroyed")
      )
        throw new Error("Select an available agent before starting a chat.");
      // Local validation must refuse before detaching any optimistic message.
      assistantTurnBody({
        conversationId: selected.current ?? uploads?.conversationId,
        agentId,
        model,
        text,
        retryOf,
        attachmentIds: uploads?.attachmentIds,
      });
      const localEpoch = epoch.current;
      const operation = {};
      const controller = new AbortController();
      streamController.current = controller;
      sendOperation.current = operation;
      sendLock.current = true;
      setSending(true);
      setError(undefined);
      setInputText(text);
      setInputAttachments(uploads?.attachments ?? []);
      let reply = emptyLiveReply();
      let id = selected.current ?? uploads?.conversationId;
      if (id) reply.conversationId = id;
      setLive(reply);
      let keepDraft = false;
      let admitted = false;
      const adoptKnownDraft = () => {
        if (!id || !mounted.current || epoch.current !== localEpoch) return;
        selected.current = id;
        setConversationId(id);
        adopt.current?.(id);
      };
      try {
        await streamAssistantTurn(
          {
            conversationId: id,
            agentId,
            model,
            text,
            retryOf,
            attachmentIds: uploads?.attachmentIds,
          },
          (event) => {
            if (!mounted.current || epoch.current !== localEpoch) return;
            reply = applyAssistantEvent(reply, event);
            if (event.event === "turn.status") {
              id = event.conversation_id;
              selected.current = id;
              setConversationId(id);
              adopt.current?.(id);
            } else if (!reply.conversationId) {
              throw new Error("Assistant did not identify the conversation.");
            }
            setLive(reply);
          },
          controller,
          () => {
            admitted = true;
            // The server has already claimed the files, even if SSE fails later.
            adoptKnownDraft();
          },
        );
      } catch (cause) {
        if (mounted.current && epoch.current === localEpoch) {
          // An explicit pre-admission client error can safely leave its text editable.
          const refused =
            cause instanceof ApiError &&
            restoreRefusedAssistantSend(cause, admitted);
          keepDraft = refused;
          if (!refused) adoptKnownDraft();
          setRecoveryRequired(!refused);
          setError(
            refused
              ? errorMessage(cause)
              : "Connection interrupted. The assistant may still be working. Refresh or open its history before sending again.",
          );
        }
      } finally {
        if (streamController.current === controller)
          streamController.current = undefined;
        if (sendOperation.current === operation) sendLock.current = false;
        try {
          if (mounted.current && epoch.current === localEpoch) {
            if (id && selected.current === id)
              await refreshHistory(id).catch(() => undefined);
            if (mounted.current && epoch.current === localEpoch)
              await refreshIndex().catch(() => undefined);
          }
        } finally {
          // Selection can change while final history/index refresh is pending.
          // Release this send's UI even then, without clearing a newer send.
          if (sendOperation.current === operation) {
            sendOperation.current = undefined;
            if (mounted.current) {
              setSending(false);
              if (epoch.current === localEpoch) {
                setLive(undefined);
                setInputText(undefined);
                setInputAttachments([]);
              }
            }
          }
        }
      }
      return !keepDraft;
    },
    [agentId, agent, model, recoveryRequired, refreshHistory, refreshIndex],
  );

  useEffect(() => {
    const id = conversationId;
    if (
      !active ||
      !id ||
      sending ||
      loadingOlder ||
      history?.conversation.active_turn ||
      !history
    )
      return;
    const queued = continuations.current.get(id);
    if (!queued?.length) return;
    continuations.current.delete(id);
    if (history.messages.at(-1)?.error_code === "cancelled") return;
    void send(queued.map(continuationText).join("\n")).catch((cause) =>
      setError(errorMessage(cause)),
    );
  }, [active, conversationId, history, sending, loadingOlder, send]);

  async function decide(acknowledgementId: string, choice: "allow" | "deny") {
    const id = selected.current;
    if (!id || decisionLock.current) return;
    if (Date.now() - lastDecision.current < 750)
      throw new Error("Please wait a moment before deciding again.");
    lastDecision.current = Date.now();
    decisionLock.current = true;
    setDeciding(acknowledgementId);
    setError(undefined);
    try {
      const row = await assistantApi.decide(id, acknowledgementId, choice);
      if (!mounted.current || selected.current !== id) return;
      historySequence.current += 1;
      if (choice === "allow" && needsClientContinuation(row)) {
        continuations.current.set(id, [
          ...(continuations.current.get(id) ?? []),
          row,
        ]);
      }
      await refreshHistory(id);
      await refreshIndex();
    } catch (cause) {
      if (mounted.current && selected.current === id)
        setError(errorMessage(cause));
      // Modal confirmations also need the failure inside their own visible UI.
      throw cause;
    } finally {
      decisionLock.current = false;
      if (mounted.current) setDeciding(undefined);
    }
  }

  async function stop() {
    const id = selected.current;
    if (!id || mutating) return;
    continuations.current.delete(id);
    setMutating(true);
    try {
      await assistantApi.stop(id);
      if (mounted.current && selected.current === id) await refreshHistory(id);
    } catch (cause) {
      if (mounted.current && selected.current === id)
        setError(errorMessage(cause));
    } finally {
      if (mounted.current) setMutating(false);
    }
  }

  async function rename(title: string) {
    const id = selected.current;
    if (!id || mutating) return false;
    setMutating(true);
    setError(undefined);
    try {
      await assistantApi.rename(id, title);
      if (mounted.current && selected.current === id) {
        historySequence.current += 1;
        await refreshHistory(id);
        await refreshIndex();
      }
      return true;
    } catch (cause) {
      if (mounted.current && selected.current === id)
        setError(errorMessage(cause));
      return false;
    } finally {
      if (mounted.current) setMutating(false);
    }
  }

  async function remove() {
    const id = selected.current;
    if (!id || mutating) return;
    setMutating(true);
    setError(undefined);
    try {
      await assistantApi.delete(id);
      if (mounted.current && selected.current === id) {
        continuations.current.delete(id);
        openConversation();
        await refreshIndex();
      }
      return true;
    } catch (cause) {
      if (mounted.current && selected.current === id)
        setError(errorMessage(cause));
      return false;
    } finally {
      if (mounted.current) setMutating(false);
    }
  }

  async function loadOlder() {
    const id = selected.current;
    const before = latestHistory.current?.before_seq;
    if (!id || !before || loadingOlder) return;
    const localEpoch = epoch.current;
    const operation = reopenSequence.current;
    setLoadingOlder(true);
    try {
      await refreshHistory(id, before);
    } catch (cause) {
      if (
        mounted.current &&
        selected.current === id &&
        epoch.current === localEpoch &&
        reopenSequence.current === operation
      )
        setError(errorMessage(cause));
    } finally {
      if (
        mounted.current &&
        selected.current === id &&
        epoch.current === localEpoch
      )
        setLoadingOlder(false);
    }
  }

  return {
    agentId,
    agent,
    agents: agents.data?.agents ?? [],
    agentsLoading: agents.isPending,
    agentsError: agents.isError ? errorMessage(agents.error) : undefined,
    reloadAgents: () => agents.refetch(),
    indexedAgentId,
    conversationId,
    conversations,
    history,
    models,
    model,
    setModel,
    live,
    inputText,
    inputAttachments,
    sending,
    loading,
    loadingOlder,
    loadingIndex,
    deciding,
    mutating,
    error,
    online,
    recoveryRequired,
    openConversation,
    recover,
    send,
    decide,
    stop,
    rename,
    remove,
    loadOlder,
    loadMoreConversations: () =>
      nextCursor
        ? refreshIndex(nextCursor).catch((cause) =>
            setError(errorMessage(cause)),
          )
        : Promise.resolve(),
    hasMoreConversations: !!nextCursor,
  };
}

function errorMessage(cause: unknown): string {
  if (cause instanceof ApiError && cause.errorKey === "turn_active")
    return "The assistant is already working on this chat. Refresh its history.";
  if (cause instanceof ApiError && cause.statusCode === 404)
    return "This chat is unavailable, or Assistant is not enabled for your account.";
  return cause instanceof Error
    ? cause.message
    : "Could not reach the assistant. Try again.";
}
