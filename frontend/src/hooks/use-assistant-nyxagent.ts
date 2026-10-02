import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import type {
  NyxAgentAcknowledgement,
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
) {
  const actorId = currentCreditsActor();
  const onFailed = (id: string, turnId: string, code: string) =>
    notifyTurnCredits(id, turnId, code, actorId);
  return conversationId || !agent
    ? nyxAgentTransport.send(conversationId, text, onAdopt, onFailed)
    : nyxAgentTransport.send(undefined, text, onAdopt, onFailed, { agent });
}

/** The user-visible turn that resumes the assistant after an allowed card. */
export function continuationText(acknowledgement: NyxAgentAcknowledgement): string {
  switch (acknowledgement.kind) {
    case "service":
      return `Approved: this chat may use ${
        acknowledgement.service_name ?? acknowledgement.service_slug ?? "the service"
      }. Continue.`;
    case "operations":
      return `Approved: ${acknowledgement.summary} Continue within the updated operation scope.`;
    case "account":
      return "Approved: account management for this chat. Continue.";
    case "action":
      return `Confirmed: ${acknowledgement.summary} (acknowledgement_id ${acknowledgement.id}). Retry it now.`;
  }
}

/** One continuation for every card allowed while a turn was running. */
export function continuationForAll(acknowledgements: readonly NyxAgentAcknowledgement[]): string {
  return acknowledgements.map(continuationText).join("\n");
}

export function useNyxAgentAssistantChat({
  selectedConversationId,
  onConversationAdopted,
  enabled = true,
  threadsAgentId,
  draftAgent,
}: {
  readonly selectedConversationId?: string;
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
  useNyxAgentLive(enabled);
  const live = useNyxAgentLiveConnected();
  const threadsKey = nyxBotQueryKeys.threads(userId);
  const agentsKey = nyxBotQueryKeys.agents(userId);
  const historyKey = nyxBotQueryKeys.history(userId, selectedConversationId);
  const threads = useQuery({
    queryKey: nyxBotQueryKeys.threads(userId, threadsAgentId),
    queryFn: () => nyxAgentTransport.list(threadsAgentId),
    enabled: enabled && Boolean(userId && threadsAgentId),
  });
  const agents = useNyxBotAgents(enabled);
  const selected = nyxAgentTransport.getConversation(selectedConversationId);
  // A thread whose agent NyxID (not this page) set to work: poll its transcript.
  const selectedAgentRunning = agents.data?.agents.some(
    (agent) => agent.id === selected?.agent?.id && agent.status === "running",
  );
  const history = useQuery({
    queryKey: historyKey,
    queryFn: async () => {
      const previous = queryClient.getQueryData<NyxAgentHistory>(historyKey);
      const page = await nyxAgentTransport.history(selectedConversationId!);
      if (
        (previous?.conversation.active_turn && !page.conversation.active_turn) ||
        previous?.conversation.pending_acknowledgements !==
          page.conversation.pending_acknowledgements ||
        // Something the thread waited for happened: NyxID resumes it.
        (previous?.waiting.length ?? 0) > page.waiting.length
      ) {
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: threadsKey }),
          queryClient.invalidateQueries({ queryKey: agentsKey }),
        ]);
      }
      return page;
    },
    enabled: enabled && Boolean(userId && selectedConversationId),
    retry: false,
    // Polling during a live turn also carries the turn's tool activity.
    // NyxID pushes changes over the live stream; these polls are the
    // fallback without it and a slow backstop with it.
    refetchInterval: (query) =>
      livePollInterval(
        streaming ||
          selectedAgentRunning ||
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
  const send = useCallback(
    async (text: string) => {
      try {
        await liveSend(selectedConversationId, text, onConversationAdopted, draftAgent);
      } finally {
        // A new thread changes its agent's summary. (Model routing is
        // server-side, so there is no profile list to refresh.)
        await queryClient.invalidateQueries({ queryKey: agentsKey });
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [selectedConversationId, onConversationAdopted, queryClient, userId, draftAgent?.id],
  );
  const stop = useCallback(async () => {
    if (selectedConversationId) await nyxAgentTransport.stop(selectedConversationId);
  }, [selectedConversationId]);

  // When any agent starts, settles or raises a request, NyxID may have woken
  // the open thread (NyxBot hears back from its specialists), so re-read it
  // and the agent's thread list.
  const agentsSignature = agents.data?.agents
    .map(
      (agent) =>
        `${agent.id}:${agent.status}:${String(agent.pending_requests.length)}:${String(
          agent.last_reply?.seq ?? 0,
        )}`,
    )
    .join(",");
  const seenAgentsSignature = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (agentsSignature === undefined) return;
    const previous = seenAgentsSignature.current;
    seenAgentsSignature.current = agentsSignature;
    if (previous === undefined || previous === agentsSignature) return;
    void queryClient.invalidateQueries({ queryKey: threadsKey });
    if (selectedConversationId) void queryClient.invalidateQueries({ queryKey: historyKey });
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
    mutationFn: async ({ id, choice }: { id: string; choice: "allow" | "deny" }) => {
      if (!selectedConversationId) throw new Error("Choose a conversation first.");
      const now = Date.now();
      if (now - lastDecision.current < 750) {
        throw new Error("Please wait a moment before deciding again.");
      }
      lastDecision.current = now;
      return nyxAgentTransport.decide(selectedConversationId, id, choice);
    },
    onSuccess: (acknowledgement, { choice }) => {
      // An allowed card resumes the assistant: the refusal told it to retry after
      // approval, and it cannot wait for the decision inside its own turn. A
      // specialist's request routed to NyxBot is resumed by the server itself.
      if (
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
      void send(continuationText(acknowledgement)).catch(() => undefined);
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: historyKey }),
        queryClient.invalidateQueries({ queryKey: threadsKey }),
        queryClient.invalidateQueries({ queryKey: agentsKey }),
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
    conversations:
      enabled && threadsAgentId ? nyxAgentTransport.getConversations(threadsAgentId) : [],
    threadsLoaded: threads.isSuccess,
    /** The selected thread's row, once loaded. */
    conversation: selected,
    session: nyxAgentTransport.session(selectedConversationId),
    isStreaming: nyxAgentTransport.isRunning(selectedConversationId),
    isLoading: history.isLoading,
    error: history.error?.message ?? threads.error?.message ?? agents.error?.message,
    send,
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
