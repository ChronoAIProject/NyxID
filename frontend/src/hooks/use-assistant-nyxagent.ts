import { useCallback, useRef, useSyncExternalStore } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import type { NyxAgentAccessMode, NyxAgentHistory } from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";

export function useNyxAgentAssistantChat({
  selectedConversationId,
  onConversationAdopted,
  enabled = true,
}: {
  readonly selectedConversationId?: string;
  readonly onConversationAdopted: (id: string) => void;
  readonly enabled?: boolean;
}) {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  useSyncExternalStore(
    nyxAgentTransport.subscribe,
    nyxAgentTransport.getRevision,
    nyxAgentTransport.getRevision,
  );
  const indexKey = ["assistant", "nyxagent", userId, "index"];
  const historyKey = ["assistant", "nyxagent", userId, "history", selectedConversationId];
  const index = useQuery({
    queryKey: indexKey,
    queryFn: () => nyxAgentTransport.list(),
    enabled: enabled && Boolean(userId),
  });
  const history = useQuery({
    queryKey: historyKey,
    queryFn: async () => {
      const previous = queryClient.getQueryData<NyxAgentHistory>(historyKey);
      const page = await nyxAgentTransport.history(selectedConversationId!);
      if (
        (previous?.conversation.active_turn && !page.conversation.active_turn) ||
        previous?.conversation.pending_acknowledgements !==
          page.conversation.pending_acknowledgements
      ) {
        await queryClient.invalidateQueries({ queryKey: indexKey });
      }
      return page;
    },
    enabled: enabled && Boolean(userId && selectedConversationId),
    retry: false,
    refetchInterval: (query) =>
      query.state.data?.conversation.active_turn ||
      query.state.data?.acknowledgements.some((row) => row.status === "pending")
        ? 2000
        : false,
  });
  const models = useQuery({
    queryKey: ["assistant", "nyxagent", userId, "models"],
    queryFn: () => nyxAgentTransport.models(),
    enabled: enabled && Boolean(userId),
    staleTime: 60_000,
  });
  const send = useCallback(
    async (text: string) => {
      try {
        await nyxAgentTransport.send(selectedConversationId, text, onConversationAdopted);
      } finally {
        // A first turn may have provisioned the credential needed for profile discovery.
        await queryClient.invalidateQueries({
          queryKey: ["assistant", "nyxagent", userId, "models"],
        });
      }
    },
    [selectedConversationId, onConversationAdopted, queryClient, userId],
  );
  const stop = useCallback(async () => {
    if (selectedConversationId) await nyxAgentTransport.stop(selectedConversationId);
  }, [selectedConversationId]);

  const mode = useMutation({
    mutationFn: (value: NyxAgentAccessMode) =>
      nyxAgentTransport.setAccessMode(selectedConversationId, value),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: historyKey }),
        queryClient.invalidateQueries({ queryKey: indexKey }),
      ]);
    },
  });
  const lastDecision = useRef(Number.NEGATIVE_INFINITY);
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
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: historyKey }),
        queryClient.invalidateQueries({ queryKey: indexKey }),
      ]);
    },
  });

  return {
    conversations: enabled ? nyxAgentTransport.getConversations() : [],
    session: nyxAgentTransport.session(selectedConversationId),
    isStreaming: nyxAgentTransport.isRunning(selectedConversationId),
    isLoading: history.isLoading,
    error: history.error?.message ?? index.error?.message,
    send,
    stop,
    acknowledgements: nyxAgentTransport.getHistory(selectedConversationId)?.acknowledgements ?? [],
    decideAcknowledgement: decision.mutateAsync,
    decidingAcknowledgement: decision.isPending ? decision.variables?.id : undefined,
    deleteConversation: (id: string) => nyxAgentTransport.delete(id),
    renameConversation: (id: string, title: string) => nyxAgentTransport.rename(id, title),
    accessMode: nyxAgentTransport.getAccessMode(selectedConversationId),
    setAccessMode: mode.mutateAsync,
    changingAccessMode: mode.isPending,
    model: nyxAgentTransport.getModel(selectedConversationId),
    setModel: (model: string) => nyxAgentTransport.setModel(model),
    models: models.data ?? [{ id: "nyxagent/chat", label: "chat" }],
    beforeSeq: nyxAgentTransport.getHistory(selectedConversationId)?.before_seq,
    loadOlder: async () => {
      const before = nyxAgentTransport.getHistory(selectedConversationId)?.before_seq;
      if (selectedConversationId && before) {
        await nyxAgentTransport.history(selectedConversationId, before);
      }
    },
  };
}
