import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import {
  useApprovalRequests,
  useNotificationSettings,
} from "@/hooks/use-approvals";
import {
  toAssistantApprovalEntry,
  type AssistantApprovalEntry,
} from "@/lib/assistant/approvals";
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
} as const;

// Live-update cadence for real approval data. Pending requests are
// time-sensitive (they expire), so they poll faster than history.
const PENDING_APPROVALS_POLL_MS = 5_000;
const APPROVAL_HISTORY_POLL_MS = 15_000;
// One page comfortably above anything a single user accumulates as
// simultaneously-pending; the badge uses the server-side total anyway.
const PENDING_APPROVALS_PAGE_SIZE = 50;
const APPROVAL_HISTORY_PAGE_SIZE = 20;

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

/**
 * Pending approval requests, polled so requests raised (or decided) from
 * anywhere — agents hitting the proxy, Telegram, mobile, another tab —
 * appear live. Shares its query cache with the sidebar badge.
 */
function usePendingApprovalRequests() {
  return useApprovalRequests(1, PENDING_APPROVALS_PAGE_SIZE, "pending", {
    refetchIntervalMs: PENDING_APPROVALS_POLL_MS,
  });
}

/**
 * Real approvals for the assistant Approvals view: pending requests plus
 * recent decided history, both mapped into assistant approval-card blocks.
 */
export function useAssistantApprovals() {
  const settings = useNotificationSettings();
  const pendingQuery = usePendingApprovalRequests();
  const historyQuery = useApprovalRequests(
    1,
    APPROVAL_HISTORY_PAGE_SIZE,
    undefined,
    { refetchIntervalMs: APPROVAL_HISTORY_POLL_MS },
  );

  // Grant-mode approvals create a reusable grant with the user-configured
  // expiry; surface that duration on the card so "approve" is informed.
  const grantDurationSec =
    settings.data !== undefined
      ? settings.data.grant_expiry_days * 86_400
      : null;

  const pending: AssistantApprovalEntry[] = (
    pendingQuery.data?.requests ?? []
  ).map((request) => toAssistantApprovalEntry(request, grantDurationSec));
  const history: AssistantApprovalEntry[] = (
    historyQuery.data?.requests ?? []
  )
    .filter((request) => request.status !== "pending")
    .map((request) => toAssistantApprovalEntry(request, grantDurationSec));

  return {
    pending,
    history,
    isLoading: pendingQuery.isLoading || historyQuery.isLoading,
    isError: pendingQuery.isError || historyQuery.isError,
    refetch: () => {
      void pendingQuery.refetch();
      void historyQuery.refetch();
    },
  };
}

/**
 * Sidebar workspace counts. Artifacts still come from the mock store
 * (TODO(api-pass): back with the artifacts listing); the pending-approvals
 * badge is the real server-side total from `/approvals/requests`.
 */
export function useWorkspaceCounts() {
  const artifactsQuery = useQuery({
    queryKey: assistantKeys.workspace,
    queryFn: () => assistantMockStore.workspaceCounts(),
  });
  const pendingQuery = usePendingApprovalRequests();
  return {
    data: {
      artifacts: artifactsQuery.data?.artifacts ?? 0,
      pendingApprovals: pendingQuery.data?.total ?? 0,
    },
  };
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
