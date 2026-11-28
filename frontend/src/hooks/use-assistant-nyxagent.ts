import { ApiError } from "@/lib/api-client";
import { historyErrorMessage, HISTORY_UNSUPPORTED } from "@/schemas/assistant-history-compat";
import type { UploadedMessage } from "@/lib/assistant/uploads";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { getAssistantIdentityRevision } from "@/lib/assistant/identity";
import type {
  AssistantAgent,
  NyxAgentConversation,
  NyxAgentConversationAgent,
  NyxAgentHistory,
} from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import { isCmaOperationPending } from "@/schemas/cma-operation";
import { useDecideApproval } from "@/hooks/use-approvals";
import { nyxBotQueryKeys, useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { useNyxAgentLive } from "@/hooks/use-nyxagent-live";
import { livePollInterval, useNyxAgentLiveConnected } from "@/hooks/use-nyxagent-live-status";
import { liveSend, notifyTurnCredits } from "@/lib/assistant/nyxagent-continuation";
import { nyxAgentDecisions, type AcknowledgementDecision, type DecisionSource, isCurrentDecisionActor } from "@/lib/assistant/nyxagent-decisions";
export { continuationText, continuationForAll } from "@/lib/assistant/nyxagent-continuation";

/**
 * Every mounted view sees the same agents change; only the first to see it
 * refreshes the thread lists, so two views never fetch them twice.
 */
const claimedAgentsChanges = new Map<string, string>();
function claimAgentsChange(userId: string, signature: string): boolean {
  const previous = claimedAgentsChanges.get(userId);
  if (previous === signature) return false;
  claimedAgentsChanges.set(userId, signature);
  return previous !== undefined;
}

/** What changes when an agent starts, settles or raises a request. */
function agentsSignatureOf(agents: readonly AssistantAgent[] | undefined): string | undefined {
  return agents
    ?.map(
      (agent) =>
        `${agent.id}:${agent.status}:${String(agent.pending_requests.length)}:${String(
          agent.last_reply?.seq ?? 0,
        )}`,
    )
    .join(",");
}

const NO_THREADS: readonly NyxAgentConversation[] = [];

/**
 * `agentId`'s threads, newest first. Re-renders only when those rows change,
 * not on every streamed token.
 */
export function useNyxAgentThreads({
  agentId,
  enabled = true,
}: {
  readonly agentId?: string;
  readonly enabled?: boolean;
}) {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  useNyxAgentLive(enabled);
  const threads = useQuery({
    queryKey: nyxBotQueryKeys.threads(userId, agentId),
    queryFn: () => nyxAgentTransport.list(agentId),
    enabled: enabled && Boolean(userId && agentId),
  });
  const agents = useNyxBotAgents(enabled);
  const signature = agentsSignatureOf(agents.data?.agents);
  useEffect(() => {
    if (!userId || signature === undefined || !claimAgentsChange(userId, signature)) return;
    void queryClient.invalidateQueries(
      { queryKey: nyxBotQueryKeys.threads(userId) },
      { cancelRefetch: false },
    );
  }, [queryClient, signature, userId]);
  const snapshot = useRef(NO_THREADS);
  const conversations = useSyncExternalStore(nyxAgentTransport.subscribe, () => {
    const next = enabled && agentId ? nyxAgentTransport.getConversations(agentId) : NO_THREADS;
    const previous = snapshot.current;
    if (next.length === previous.length && next.every((row, index) => row === previous[index])) {
      return previous;
    }
    snapshot.current = next;
    return next;
  });
  return { conversations, threadsLoaded: threads.isSuccess, threadsError: threads.error, agents };
}

export function useNyxAgentAssistantChat({
  selectedConversationId,
  onConversationAdopted,
  enabled = true,
  threadsAgentId,
  draftAgent,
  reviewCardId,
}: {
  readonly selectedConversationId?: string;
  readonly reviewCardId?: string;
  readonly onConversationAdopted: (id: string) => void;
  readonly enabled?: boolean;
  /** List this agent's threads (for the sidebar and landing). */
  readonly threadsAgentId?: string;
  /** The agent a new thread starts with; the server defaults to NyxBot. */
  readonly draftAgent?: NyxAgentConversationAgent;
}) {
  const userId = useAuthStore((state) => state.user?.id);
  const identityRevision = getAssistantIdentityRevision();
  const queryClient = useQueryClient();
  const transportRevision = useSyncExternalStore(
    nyxAgentTransport.subscribe,
    nyxAgentTransport.getRevision,
    nyxAgentTransport.getRevision,
  );
  const streaming = nyxAgentTransport.isRunning(selectedConversationId);
  // A new thread's first send with uploads goes to the server draft that holds
  // them. Show that draft's live turn (and its images) at once, as a new thread
  // without uploads is shown, rather than only once the page adopts the id.
  const [sendingDraft, setSendingDraft] = useState<string>();
  const viewId =
    selectedConversationId ??
    (sendingDraft && nyxAgentTransport.isRunning(sendingDraft) ? sendingDraft : undefined);
  const viewSession = nyxAgentTransport.session(viewId);
  const live = useNyxAgentLiveConnected();
  const threadsKey = nyxBotQueryKeys.threads(userId);
  const agentsKey = nyxBotQueryKeys.agents(userId);
  const historyKey = [...nyxBotQueryKeys.history(userId, selectedConversationId), reviewCardId ?? null];
  const threads = useNyxAgentThreads({ agentId: threadsAgentId, enabled });
  const agents = threads.agents;
  const selected = nyxAgentTransport.getConversation(selectedConversationId);
  const history = useQuery({
    queryKey: historyKey,
    queryFn: () =>
      nyxAgentTransport.history(selectedConversationId!, undefined, reviewCardId ?? null),
    enabled: enabled && Boolean(userId && selectedConversationId),
    retry: false,
    // Polling during a live turn also carries the turn's tool activity.
    // NyxID pushes changes over the live stream; these polls are the
    // fallback without it and a slow backstop with it. A turn NyxID (not
    // this page) starts here reaches the thread row first: `streaming`.
    refetchInterval: (query) => {
      // An owner can decide a proxy approval outside this thread.
      const oldestDeadline = Date.now() - 15_000;
      if (query.state.data?.approvals.some((row) =>
        (!row.status || row.status === "pending") && Date.parse(row.expires_at) >= oldestDeadline
      )) return 2000;
      return streaming || nyxAgentTransport.hasUnsettledInterruption(selectedConversationId) || query.state.data?.conversation.active_turn
        ? (live ? 5000 : 2000)
        : query.state.data?.cma_operations.some(isCmaOperationPending)
          ? 5000
          : livePollInterval(
              query.state.data?.acknowledgements.some((row) => row.status === "pending")
                ? 2000
                : // Waiting on something outside the chat: notice when it happens.
                  (query.state.data?.waiting.length ?? 0) > 0
                  ? 10_000
                  : false,
              live,
            );
    },
  });
  const historyDenied = history.error instanceof ApiError && (history.error.status === 403 || history.error.status === 404);
  useEffect(() => {
    if (historyDenied) void queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) });
  }, [historyDenied, queryClient, userId]);
  // A settled turn, a decided card or a resolved wait changes the agent's
  // summary and its thread row.
  const seenHistory = useRef<NyxAgentHistory | undefined>(undefined);
  useEffect(() => {
    const previous = seenHistory.current;
    const page = history.data;
    seenHistory.current = page;
    if (!previous || !page || previous.conversation.id !== page.conversation.id) return;
    const pendingChanged =
      previous.conversation.pending_acknowledgements !==
      page.conversation.pending_acknowledgements;
    if (pendingChanged)
      void queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.activity(userId) }, { cancelRefetch: false });
    if (
      (previous.conversation.active_turn && !page.conversation.active_turn) ||
      pendingChanged ||
      // Something the thread waited for happened: NyxID resumes it.
      previous.waiting.length > page.waiting.length
    ) {
      void queryClient.invalidateQueries({ queryKey: threadsKey }, { cancelRefetch: false });
      void queryClient.invalidateQueries({ queryKey: agentsKey }, { cancelRefetch: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [history.data]);
  const send = useCallback(
    async (text: string, uploads?: UploadedMessage, model?: string) => {
      let draft: string | undefined;
      try {
        const id = selectedConversationId ?? uploads?.conversationId;
        if (id && !selectedConversationId) {
          draft = id;
          setSendingDraft(id);
        }
        await liveSend(
          id, text, onConversationAdopted, draftAgent,
          uploads?.attachmentIds, uploads?.attachments, Boolean(id && !selectedConversationId),
          undefined, model,
        );
      } finally {
        if (draft) setSendingDraft((current) => (current === draft ? undefined : current));
        // Provisioning on the first send can make the live model list available.
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: agentsKey }, { cancelRefetch: false }),
          queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.models(userId) }),
        ]);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [selectedConversationId, onConversationAdopted, queryClient, userId, draftAgent?.id],
  );
  const stop = useCallback(async () => {
    if (selectedConversationId) await nyxAgentTransport.stop(selectedConversationId);
  }, [selectedConversationId]);

  // When any agent starts, settles or raises a request, NyxID may have woken
  // the open thread (NyxBot hears back from its specialists), so re-read it.
  // `useNyxAgentThreads` refreshes the thread lists.
  const agentsSignature = agentsSignatureOf(agents.data?.agents);
  const seenAgentsSignature = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (agentsSignature === undefined) return;
    const previous = seenAgentsSignature.current;
    seenAgentsSignature.current = agentsSignature;
    if (previous === undefined || previous === agentsSignature) return;
    if (selectedConversationId) {
      void queryClient.invalidateQueries({ queryKey: historyKey }, { cancelRefetch: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentsSignature]);

  const lastDecision = useRef(Number.NEGATIVE_INFINITY);
  // Observation belongs to the view; accepted decisions belong to the transport.
  useEffect(() => {
    if (!enabled && selectedConversationId && userId === useAuthStore.getState().user?.id &&
      identityRevision === getAssistantIdentityRevision()) nyxAgentDecisions.refuse(selectedConversationId);
  }, [enabled, selectedConversationId, userId, identityRevision]);
  useEffect(() => {
    if (!enabled || !userId || !selectedConversationId) return;
    return nyxAgentDecisions.observe(selectedConversationId);
  }, [enabled, userId, selectedConversationId]);
  useEffect(() => nyxAgentDecisions.refresh(), [streaming, transportRevision]);
  const decision = useMutation({
    mutationFn: async ({ id, choice, acceptReducedScope, source }: AcknowledgementDecision & { source: DecisionSource }) => {
      if (!source.conversationId) throw new Error("Choose a conversation first.");
      if (!nyxAgentDecisions.isCurrentSource(source)) throw new DOMException("Identity changed", "AbortError");
      const now = Date.now();
      if (now - lastDecision.current < 750) {
        throw new Error("Please wait a moment before deciding again.");
      }
      lastDecision.current = now;
      const acknowledgement = await nyxAgentTransport.decide(source.conversationId, id, choice, acceptReducedScope);
      nyxAgentDecisions.complete(source, choice, acknowledgement);
      return acknowledgement;
    },
    onSettled: async (_acknowledgement, _error, { source }) => {
      if (!isCurrentDecisionActor(source)) return;
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.history(source.owner, source.conversationId) }, { cancelRefetch: false }),
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.threads(source.owner) }, { cancelRefetch: false }),
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(source.owner) }, { cancelRefetch: false }),
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.activity(source.owner) }, { cancelRefetch: false }),
      ]);
    },
  });

  // Turns seen running on this page. Only their later failures may prompt, so
  // a transcript that loads already failed (or older pages) never does.
  const observedTurns = useRef(new Set<string>());
  useEffect(() => {
    const id = selectedConversationId;
    if (!id) return;
    const messages = nyxAgentTransport.getHistory(id)?.messages ?? [];
    const activeTurnId = nyxAgentTransport.getActiveTurnId(id);
    // A turn whose reply is already settled in history was never seen running
    // here, whatever a stale snapshot claims.
    const settled = messages.some(
      (message) => message.turn_id === activeTurnId && message.role === "assistant",
    );
    if (activeTurnId && !settled) observedTurns.current.add(`${id}:${activeTurnId}`);
    for (const message of messages) {
      if (
        message.status === "failed" &&
        observedTurns.current.has(`${id}:${message.turn_id}`)
      ) {
        notifyTurnCredits(id, message.turn_id, message.error_code, userId ?? null);
      }
    }
  });

  const approvalDecision = useDecideApproval();
  const decideApproval = useCallback(
    async (requestId: string, approved: boolean) => {
      try {
        await approvalDecision.mutateAsync({ requestId, approved });
      } finally {
        await queryClient.invalidateQueries({ queryKey: historyKey });
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [approvalDecision.mutateAsync, queryClient, selectedConversationId, userId],
  );

  return {
    approvals: nyxAgentTransport.getHistory(selectedConversationId)?.approvals ?? [],
    /** What the thread is waiting for outside the chat. */
    waiting: nyxAgentTransport.getHistory(selectedConversationId)?.waiting ?? [],
    decideApproval,
    /** `threadsAgentId`'s threads, newest first. */
    conversations: threads.conversations,
    threadsLoaded: threads.threadsLoaded,
    /** The selected thread's row, once loaded. */
    conversation: selected,
    // Until adoption the page treats the draft as a new thread (no conversation id).
    session: viewId === selectedConversationId
      ? viewSession
      : { ...viewSession, clientId: "nyxagent-draft", conversationId: undefined },
    continuations: nyxAgentTransport.continuations(viewId),
    isStreaming: nyxAgentTransport.isRunning(viewId),
    historyDenied,
    isLoading: history.isLoading,
    interruption: nyxAgentTransport.getInterruption(viewId),
    error: historyErrorMessage(history.error ?? threads.threadsError ?? agents.error)
      ?? (nyxAgentTransport.getHistory(selectedConversationId)?.unsupported_history_items ? HISTORY_UNSUPPORTED : undefined),
    send,
    retry: (id: string, messageId: string) =>
      nyxAgentTransport.retry(id, messageId, onConversationAdopted),
    checkRecovery: (id: string, messageId: string) =>
      nyxAgentTransport.checkRecovery(id, messageId),
    /** Reload the selected thread's transcript (after a context reset). */
    refreshHistory: async () => {
      if (selectedConversationId) await history.refetch();
    },
    stop,
    acknowledgements: nyxAgentTransport.getHistory(selectedConversationId)?.acknowledgements ?? [],
    // Mutation options follow rerenders. Capture authority before async middleware
    // so both the request and its callbacks retain the exact source of the click.
    decideAcknowledgement: (input: AcknowledgementDecision) => decision.mutateAsync({
      ...input,
      source: nyxAgentDecisions.capture(selectedConversationId, userId, identityRevision),
    }),
    decidingAcknowledgement: decision.isPending ? decision.variables?.id : undefined,
    deleteConversation: (id: string) => nyxAgentTransport.delete(id),
    renameConversation: (id: string, title: string) => nyxAgentTransport.rename(id, title),
    beforeSeq: nyxAgentTransport.getHistory(selectedConversationId)?.before_seq,
    loadOlder: async () => {
      const before = nyxAgentTransport.getHistory(selectedConversationId)?.before_seq;
      if (selectedConversationId && before) {
        await nyxAgentTransport.history(selectedConversationId, before);
      }
    },
  };
}
