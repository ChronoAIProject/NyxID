import type { UploadedMessage } from "@/lib/assistant/uploads";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import type {
  AssistantAgent,
  NyxAgentAcknowledgement,
  NyxAgentConversation,
  NyxAgentConversationAgent,
  NyxAgentHistory,
} from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import { useDecideApproval } from "@/hooks/use-approvals";
import { nyxBotQueryKeys, useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { useNyxAgentLive } from "@/hooks/use-nyxagent-live";
import { livePollInterval, useNyxAgentLiveConnected } from "@/hooks/use-nyxagent-live-status";
import {
  currentCreditsActor,
  isInsufficientCreditsCode,
  notifyCreditsDenied,
} from "@/lib/credits-denial";

function notifyTurnCredits(
  conversationId: string,
  turnId: string,
  code: string | null,
  actorId: string | null,
): void {
  if (!isInsufficientCreditsCode(code)) return;
  // SSE and the history poll can both deliver one failure; the key dedupes.
  notifyCreditsDenied(
    { key: `assistant:nyxagent:${conversationId}:${turnId}`, payer: "self" },
    actorId,
  );
}

/**
 * A foreground send whose live failure may open the out-of-credits dialog.
 * `agent` is only used when the send starts a new thread.
 */
function liveSend(
  conversationId: string | undefined,
  text: string,
  onAdopt: (id: string) => void,
  agent?: NyxAgentConversationAgent,
  attachmentIds?: string[],
  attachments?: readonly import("@/schemas/assistant-nyxagent").NyxAgentAttachment[],
  adoptExisting?: boolean,
  acknowledgementId?: string,
) {
  const actorId = currentCreditsActor();
  const onFailed = (id: string, turnId: string, code: string) =>
    notifyTurnCredits(id, turnId, code, actorId);
  if (attachmentIds?.length || acknowledgementId) {
    return nyxAgentTransport.send(conversationId, text, onAdopt, onFailed, {
      ...(!conversationId && agent ? { agent } : {}),
      attachmentIds,
      attachments,
      adoptExisting,
      acknowledgementId,
    });
  }
  return conversationId || !agent
    ? nyxAgentTransport.send(conversationId, text, onAdopt, onFailed)
    : nyxAgentTransport.send(undefined, text, onAdopt, onFailed, { agent });
}

/** The user-visible turn that resumes the assistant after an allowed card. */
export function continuationText(acknowledgement: NyxAgentAcknowledgement): string {
  const narration = "Then tell the user in one or two plain sentences what you did and the result.";
  const instruction = continuationInstruction(acknowledgement);
  return `${instruction} ${narration}`;
}

function continuationInstruction(acknowledgement: NyxAgentAcknowledgement): string {
  switch (acknowledgement.kind) {
    case "service":
      return `Approved: this chat may use ${
        acknowledgement.service_name ?? acknowledgement.service_slug ?? "the service"
      }. Continue.`;
    case "skills":
      return `Skill proposal reviewed: ${acknowledgement.summary} NyxBot will resolve the exact version and apply the governing mode to attach it (automatic in Full access, otherwise an owner card). Do not request the skill again.`;
    case "operations":
      return `Approved: ${acknowledgement.summary} Continue within the updated operation scope.`;
    case "machine":
    case "saved_login":
      return `Approved: ${acknowledgement.summary} Continue within the updated machine and saved-login grants.`;
    case "account":
      return "Approved: account management for this chat. Continue.";
    case "action":
      if (acknowledgement.authored_skill) return `The skill publication was allowed: ${acknowledgement.summary}. Check the attached skills and tell the user which skill/version was attached to which agent. Do not publish it again.`;
      return `Confirmed: ${acknowledgement.summary} (acknowledgement_id ${acknowledgement.id}). Retry it now.`;
  }
}

/** One continuation for every card allowed while a turn was running. */
export function continuationForAll(acknowledgements: readonly NyxAgentAcknowledgement[]): string {
  return acknowledgements.map(continuationText).join("\n");
}

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
  const queryClient = useQueryClient();
  useSyncExternalStore(
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
    refetchInterval: (query) =>
      livePollInterval(
        streaming ||
          query.state.data?.conversation.active_turn ||
          query.state.data?.acknowledgements.some((row) => row.status === "pending") ||
          (query.state.data?.approvals.length ?? 0) > 0
          ? 2000
          : // Waiting on something outside the chat: notice when it happens.
            (query.state.data?.waiting.length ?? 0) > 0
            ? 10_000
            : false,
        live,
      ),
  });
  // A settled turn, a decided card or a resolved wait changes the agent's
  // summary and its thread row.
  const seenHistory = useRef<NyxAgentHistory | undefined>(undefined);
  useEffect(() => {
    const previous = seenHistory.current;
    const page = history.data;
    seenHistory.current = page;
    if (!previous || !page || previous.conversation.id !== page.conversation.id) return;
    if (
      (previous.conversation.active_turn && !page.conversation.active_turn) ||
      previous.conversation.pending_acknowledgements !== page.conversation.pending_acknowledgements ||
      // Something the thread waited for happened: NyxID resumes it.
      previous.waiting.length > page.waiting.length
    ) {
      void queryClient.invalidateQueries({ queryKey: threadsKey }, { cancelRefetch: false });
      void queryClient.invalidateQueries({ queryKey: agentsKey }, { cancelRefetch: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [history.data]);
  const send = useCallback(
    async (text: string, uploads?: UploadedMessage) => {
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
        );
      } finally {
        if (draft) setSendingDraft((current) => (current === draft ? undefined : current));
        // A new thread changes its agent's summary. (Model routing is
        // server-side, so there is no profile list to refresh.)
        await queryClient.invalidateQueries({ queryKey: agentsKey }, { cancelRefetch: false });
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
  // Cards allowed while their conversation's turn was still running. The
  // running turn cannot observe the decision (NyxAgent ends a turn on a card and
  // answers same-turn repeats locally), so the continuation waits for settlement.
  const pendingContinuations = useRef(
    new Map<string, { owner: string | undefined; acknowledgements: NyxAgentAcknowledgement[] }>(),
  );
  const decision = useMutation({
    mutationFn: async ({ id, choice, acceptReducedScope }: { id: string; choice: "allow" | "deny"; acceptReducedScope?: boolean }) => {
      if (!selectedConversationId) throw new Error("Choose a conversation first.");
      const now = Date.now();
      if (now - lastDecision.current < 750) {
        throw new Error("Please wait a moment before deciding again.");
      }
      lastDecision.current = now;
      return nyxAgentTransport.decide(selectedConversationId, id, choice, acceptReducedScope);
    },
    onSuccess: (acknowledgement, { choice }) => {
      // An allowed card resumes the assistant: the refusal told it to retry after
      // approval, and it cannot wait for the decision inside its own turn. A
      // specialist's request routed to NyxBot is resumed by the server itself.
      if (
        acknowledgement.continuation_owner === "server" ||
        acknowledgement.decider === "orchestrator" ||
        acknowledgement.trigger_run_id
      )
        return;
      if (choice !== "allow" || !selectedConversationId) return;
      if (nyxAgentTransport.isRunning(selectedConversationId)) {
        const queued = pendingContinuations.current.get(selectedConversationId);
        pendingContinuations.current.set(selectedConversationId, {
          owner: userId,
          acknowledgements: [...(queued?.acknowledgements ?? []), acknowledgement],
        });
        return;
      }
      void liveSend(selectedConversationId, continuationText(acknowledgement), onConversationAdopted,
        undefined, undefined, undefined, undefined, acknowledgement.id).catch(() => undefined);
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: historyKey }, { cancelRefetch: false }),
        queryClient.invalidateQueries({ queryKey: threadsKey }, { cancelRefetch: false }),
        queryClient.invalidateQueries({ queryKey: agentsKey }, { cancelRefetch: false }),
      ]);
    },
  });

  // Runs after every render: transport changes re-render this hook, so a queued
  // continuation is sent as soon as its turn has settled and history is fresh.
  useEffect(() => {
    for (const [conversationId, queued] of pendingContinuations.current) {
      if (queued.owner !== userId) {
        pendingContinuations.current.delete(conversationId);
        continue;
      }
      if (nyxAgentTransport.isRunning(conversationId)) continue;
      pendingContinuations.current.delete(conversationId);
      // Stop means the user wants the assistant to halt; do not resume it.
      const tail = nyxAgentTransport.getHistory(conversationId)?.messages.at(-1);
      if (tail?.error_code === "cancelled") continue;
      void liveSend(
        conversationId,
        continuationForAll(queued.acknowledgements),
        onConversationAdopted,
        undefined, undefined, undefined, undefined, queued.acknowledgements[0]?.id,
      ).catch(() => undefined);
    }
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
      await approvalDecision.mutateAsync({ requestId, approved });
      await queryClient.invalidateQueries({ queryKey: historyKey });
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
    isLoading: history.isLoading,
    error: history.error?.message ?? threads.threadsError?.message ?? agents.error?.message,
    send,
    retry: (id: string, messageId: string) =>
      nyxAgentTransport.retry(id, messageId, onConversationAdopted),
    checkRecovery: (id: string, messageId: string) =>
      nyxAgentTransport.checkRecovery(id, messageId),
    /** Reload the selected thread's transcript (after a context reset). */
    refreshHistory: async () => {
      if (selectedConversationId) await nyxAgentTransport.history(selectedConversationId);
    },
    stop,
    acknowledgements: nyxAgentTransport.getHistory(selectedConversationId)?.acknowledgements ?? [],
    decideAcknowledgement: decision.mutateAsync,
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
