import { useEffect } from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import {
  AssistantTurnActiveError,
  AssistantTurnCancelledError,
} from "@/lib/assistant/errors";
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

// One stable sonner id for the whole surface: refetches and the history query
// failing alongside the list update this toast instead of stacking a new one
// every poll.
const TRANSPORT_TOAST_ID = "assistant-transport-unavailable";

/**
 * Chat reaches aevatar through NyxID's `/api/v1/assistant/*` pass-through,
 * which forwards the DOWNSTREAM's status straight through. A 401 there means
 * aevatar rejected the identity NyxID forwarded — the NyxID session itself is
 * fine — so the transport opts those requests out of the global sign-out
 * (`preserveSessionOn401` in aevatar-transport) and the route stays put.
 * Without a toast that failure is invisible: the sidebar just renders empty,
 * which reads as "no chats yet" rather than "chat is down".
 */
export function describeTransportError(error: unknown): {
  readonly message: string;
  readonly description: string;
} {
  if (error instanceof ApiError && error.status === 401) {
    return {
      message: "Assistant chat is unavailable",
      description:
        "NyxID could not authenticate you to the chat backend. You are still signed in — reconnect the aevatar service and try again.",
    };
  }
  return {
    message: "Could not load your chats",
    description: "The assistant backend did not respond. Retrying shortly.",
  };
}

function useTransportErrorToast(isError: boolean, error: unknown): void {
  useEffect(() => {
    if (!isError) return;
    const { message, description } = describeTransportError(error);
    toast.error(message, { id: TRANSPORT_TOAST_ID, description });
  }, [isError, error]);
}

export function useConversations() {
  const query = useQuery({
    queryKey: assistantKeys.conversations,
    queryFn: () => assistantTransport.listConversations(),
  });
  useTransportErrorToast(query.isError, query.error);
  return query;
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

/**
 * Delete a conversation (the server runs the composite actor + history-row
 * delete). Cache surgery instead of invalidation: the transport's local
 * mirror already dropped the row, so re-reading it is authoritative and a
 * refetch cannot race the upstream history materialization back in.
 */
export function useDeleteConversation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (conversationId: string) =>
      assistantTransport.deleteConversation(conversationId),
    onSuccess: async (_result, conversationId) => {
      activeHandles.delete(conversationId);
      queryClient.removeQueries({
        queryKey: assistantKeys.history(conversationId),
      });
      queryClient.removeQueries({
        queryKey: assistantKeys.turn(conversationId),
      });
      const conversations = await assistantTransport.listConversations();
      queryClient.setQueryData<Conversation[]>(
        assistantKeys.conversations,
        () => conversations,
      );
    },
  });
}

export interface SentMessage {
  readonly conversationId: string;
  readonly handle: TurnHandle;
}

/**
 * Per-stream turn-event pump: projects transport events into the query
 * cache. One instance per streamed turn — sends and approval continuations
 * alike — so its cursor guard is scoped to that stream: a rejected
 * concurrent send never touches it, and at-least-once duplicates (e.g. a
 * late "running") cannot regress terminal state.
 */
function createTurnEventPump(
  queryClient: QueryClient,
  targetId: string,
): (event: TurnEvent) => void {
  let lastSeenCursor = 0;
  return (event) => {
    if (event.cursor <= lastSeenCursor) return;
    lastSeenCursor = event.cursor;
    const turn = turnFromEvent(event);
    if (turn) {
      queryClient.setQueryData<ActiveTurn | null>(
        assistantKeys.turn(targetId),
        () => turn,
      );
    }
    if (event.event === "turn.completed") {
      activeHandles.delete(targetId);
      // The mutation resolved when the stream STARTED, so failures
      // after that (pre-SSE rejection, truncated stream, RUN_ERROR)
      // never reject `mutateAsync` — without this toast they exist
      // only as cached turn state nothing renders, and the chat
      // looks dead again. Stable id: at-least-once event delivery
      // must not stack duplicates.
      if (event.status === "failed" && event.error) {
        toast.error("The assistant reply failed", {
          id: `assistant-turn-failed-${event.turn_id}`,
          description: event.error.message,
        });
      }
    }
    // Swallow projection failures: after a delete races a cancel-driven
    // event, the history read legitimately rejects (tombstoned id) and
    // must not surface as an unhandled rejection.
    projectTransportState(queryClient, targetId).catch(() => undefined);
  };
}

// Single-flight guard for the empty-state auto-create: two sends racing in
// before React commits the disabled/sending state must share ONE created
// conversation (the second is then rejected by the active-turn guard)
// instead of allocating two actors and streaming into both.
let pendingAutoCreate: Promise<Conversation> | null = null;

/**
 * Send a message into the bound conversation — or, when no conversation is
 * selected (the "New chat" empty state), create one first and send there.
 * Without the auto-create, the first send of a fresh account has no target
 * and must not silently do nothing; the caller navigates to the returned
 * `conversationId` to follow the new thread.
 */
export function useSendMessage(conversationId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (content: string): Promise<SentMessage> => {
      let target = conversationId;
      if (!target) {
        pendingAutoCreate ??= assistantTransport
          .createConversation()
          .finally(() => {
            pendingAutoCreate = null;
          });
        const conversation = await pendingAutoCreate;
        target = conversation.id;
        await projectTransportState(queryClient, target);
      }
      const targetId = target;
      const handle = assistantTransport.sendMessage(
        targetId,
        content,
        createTurnEventPump(queryClient, targetId),
      );
      activeHandles.set(targetId, handle);
      await projectTransportState(queryClient, targetId);
      return { conversationId: targetId, handle };
    },
  });
}

/**
 * Human-readable failure for a send that never became a turn (transport
 * threw before any turn event existed). Failures after the stream starts
 * surface inside the thread via `turn.completed`; this covers the gap
 * before that, which previously vanished into the composer's restore-text
 * catch and made the send button look dead.
 */
export function describeSendFailure(error: unknown): {
  readonly message: string;
  readonly description: string;
} {
  if (error instanceof ApiError && (error.status === 401 || error.status === 403)) {
    return {
      message: "Message not sent",
      description:
        "NyxID could not authenticate you to the chat backend. You are still signed in — reconnect the aevatar service and try again.",
    };
  }
  if (error instanceof AssistantTurnActiveError) {
    return {
      message: "Message not sent",
      description: "Wait for the current reply to finish, then try again.",
    };
  }
  return {
    message: "Message not sent",
    description:
      error instanceof Error && error.message
        ? error.message
        : "The assistant backend did not respond. Your text was kept — try again.",
  };
}

export function useCancelTurn(conversationId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (): Promise<void> => {
      if (!conversationId) return;
      // Transport-level lookup, not the handle registry: an approval
      // continuation's handle only registers after its response headers
      // arrive, and Stop must abort a request hung before that.
      assistantTransport.cancelActiveTurn(conversationId);
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
      // On the live contract the approve endpoint streams an SSE
      // continuation of the run; the pump projects its events and the
      // handle makes the stop button work during it.
      const handle = await assistantTransport.decideApproval(
        conversationId,
        blockId,
        approved,
        createTurnEventPump(queryClient, conversationId),
      );
      if (handle) activeHandles.set(conversationId, handle);
      await projectTransportState(queryClient, conversationId);
    },
    // Without this toast a failed approve POST (or an active-turn
    // rejection) is invisible: the buttons just re-enable and nothing
    // happens. A user-initiated Stop is an expected outcome, not a
    // delivery failure — no toast.
    onError: (error) => {
      if (error instanceof AssistantTurnCancelledError) return;
      toast.error("Approval was not delivered", {
        id: "assistant-approval-failed",
        description:
          error instanceof AssistantTurnActiveError
            ? "Wait for the current reply to finish, then decide again."
            : error instanceof Error && error.message
              ? error.message
              : "The assistant backend did not respond. Try again.",
      });
    },
  });
}
