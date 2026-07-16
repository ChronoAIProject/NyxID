import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { assistantMockStore } from "@/lib/assistant/mock-data";
import { assistantTransport } from "@/lib/assistant/transport";
import type {
  ActiveTurn,
  Conversation,
  ConversationHistory,
  TurnEvent,
  TurnHandle,
} from "@/types/assistant";

const ROOT = ["assistant"] as const;

export const assistantKeys = {
  conversations: [...ROOT, "conversations"] as const,
  history: (conversationId: string) =>
    [...ROOT, "history", conversationId] as const,
  turn: (conversationId: string) => [...ROOT, "turn", conversationId] as const,
  workspace: [...ROOT, "workspace"] as const,
  approvals: [...ROOT, "approvals"] as const,
} as const;

const activeHandles = new Map<string, TurnHandle>();

async function projectTransportState(
  queryClient: QueryClient,
  conversationId: string,
): Promise<void> {
  const [history, conversations] = await Promise.all([
    assistantTransport.getHistory(conversationId),
    assistantTransport.listConversations(),
  ]);
  queryClient.setQueryData<ConversationHistory>(
    assistantKeys.history(conversationId),
    () => history,
  );
  queryClient.setQueryData<Conversation[]>(
    assistantKeys.conversations,
    () => conversations,
  );
  await queryClient.invalidateQueries({ queryKey: assistantKeys.workspace });
  await queryClient.invalidateQueries({ queryKey: assistantKeys.approvals });
}

function turnFromEvent(event: TurnEvent): ActiveTurn | undefined {
  if (event.event === "turn.status") {
    return { turnId: event.turn_id, status: event.status, error: null };
  }
  if (event.event === "turn.completed") {
    return {
      turnId: event.turn_id,
      status: event.status,
      error: event.error,
    };
  }
  return undefined;
}

// TODO(api-pass): workspace counts read the mock store directly — these are
// sidebar chrome, not part of the C1 transport contract. The API pass derives
// them from real endpoints (artifacts listing, pending approvals).
export function useWorkspaceCounts() {
  return useQuery({
    queryKey: assistantKeys.workspace,
    queryFn: () => assistantMockStore.workspaceCounts(),
  });
}

// TODO(api-pass): approvals across conversations read the mock store directly;
// the API pass backs this with the real approvals listing.
export function useApprovals() {
  return useQuery({
    queryKey: assistantKeys.approvals,
    queryFn: () => assistantMockStore.listApprovals(),
  });
}

/** Decide an approval from outside its conversation (the Approvals view). */
export function useDecideApprovalFor() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      conversationId,
      blockId,
      approved,
    }: {
      readonly conversationId: string;
      readonly blockId: string;
      readonly approved: boolean;
    }): Promise<void> => {
      await assistantTransport.decideApproval(
        conversationId,
        blockId,
        approved,
      );
      await projectTransportState(queryClient, conversationId);
    },
  });
}

export function useConversations() {
  return useQuery({
    queryKey: assistantKeys.conversations,
    queryFn: () => assistantTransport.listConversations(),
  });
}

export function useConversation(conversationId: string | undefined) {
  return useQuery({
    queryKey: assistantKeys.history(conversationId ?? ""),
    queryFn: () => assistantTransport.getHistory(conversationId ?? ""),
    enabled: Boolean(conversationId),
  });
}

export function useAssistantTurn(conversationId: string | undefined) {
  return useQuery({
    queryKey: assistantKeys.turn(conversationId ?? ""),
    queryFn: async (): Promise<ActiveTurn | null> => null,
    enabled: false,
    initialData: null,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useCreateConversation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => assistantTransport.createConversation(),
    onSuccess: async (conversation) => {
      await projectTransportState(queryClient, conversation.id);
    },
  });
}

export function useSendMessage(conversationId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (content: string): Promise<TurnHandle> => {
      if (!conversationId) throw new Error("Select a conversation first.");
      // Last-seen per-turn cursor, scoped to THIS send's event stream: a
      // rejected concurrent send never touches it, and at-least-once
      // duplicates (e.g. a late "running") cannot regress terminal state.
      let lastSeenCursor = 0;
      const handle = assistantTransport.sendMessage(
        conversationId,
        content,
        (event) => {
          if (event.cursor <= lastSeenCursor) return;
          lastSeenCursor = event.cursor;
          const turn = turnFromEvent(event);
          if (turn) {
            queryClient.setQueryData<ActiveTurn | null>(
              assistantKeys.turn(conversationId),
              () => turn,
            );
          }
          if (event.event === "turn.completed") {
            activeHandles.delete(conversationId);
          }
          void projectTransportState(queryClient, conversationId);
        },
      );
      activeHandles.set(conversationId, handle);
      await projectTransportState(queryClient, conversationId);
      return handle;
    },
  });
}

export function useCancelTurn(conversationId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (): Promise<void> => {
      if (!conversationId) return;
      activeHandles.get(conversationId)?.cancel();
      activeHandles.delete(conversationId);
      await projectTransportState(queryClient, conversationId);
    },
  });
}

export function useDecideApproval(conversationId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      blockId,
      approved,
    }: {
      readonly blockId: string;
      readonly approved: boolean;
    }): Promise<void> => {
      if (!conversationId) throw new Error("Select a conversation first.");
      await assistantTransport.decideApproval(
        conversationId,
        blockId,
        approved,
      );
      await projectTransportState(queryClient, conversationId);
    },
  });
}
