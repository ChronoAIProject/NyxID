import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  ChannelConversationListResponse,
  ChannelConversationItem,
  CreateChannelConversationRequest,
  CreateDeviceConversationRequest,
  UpdateChannelConversationRequest,
} from "@/types/channels";

// ─────────────────────────────────────────────────────────────────────────────
// Query keys
// ─────────────────────────────────────────────────────────────────────────────

const CHANNEL_CONVERSATIONS_ROOT = ["channel-conversations"] as const;

export const channelConversationsQueryKeys = {
  all: CHANNEL_CONVERSATIONS_ROOT,
  list: (orgId: string | null, botId: string | null) =>
    [
      ...CHANNEL_CONVERSATIONS_ROOT,
      "list",
      orgId ?? "personal",
      botId ?? "all-bots",
    ] as const,
} as const;

// ─────────────────────────────────────────────────────────────────────────────
// Queries
// ─────────────────────────────────────────────────────────────────────────────

interface UseChannelConversationsParams {
  readonly botId?: string | null;
  /** `null` or omitted lists personal conversations. When set, lists
   *  conversations owned by the given org (admin-only). */
  readonly orgId?: string | null;
}

export function useChannelConversations(params: UseChannelConversationsParams = {}) {
  const orgId = params.orgId ?? null;
  const botId = params.botId ?? null;
  return useQuery({
    queryKey: channelConversationsQueryKeys.list(orgId, botId),
    queryFn: async (): Promise<readonly ChannelConversationItem[]> => {
      const qs: string[] = [];
      if (botId) qs.push(`bot_id=${encodeURIComponent(botId)}`);
      if (orgId) qs.push(`org_id=${encodeURIComponent(orgId)}`);
      const path =
        qs.length === 0
          ? "/channel-conversations"
          : `/channel-conversations?${qs.join("&")}`;
      const res = await api.get<ChannelConversationListResponse>(path);
      return res.conversations;
    },
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Mutations
// ─────────────────────────────────────────────────────────────────────────────

export function useCreateChannelConversation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateChannelConversationRequest,
    ): Promise<ChannelConversationItem> => {
      return api.post<ChannelConversationItem>(
        "/channel-conversations",
        data,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: CHANNEL_CONVERSATIONS_ROOT,
      });
      void queryClient.invalidateQueries({ queryKey: ["channel-bots"] });
    },
  });
}

/**
 * Create a device channel (HTTP Event Gateway, NyxID#221). Device channels
 * are not backed by a bot and accept events via
 * `POST /api/v1/channel-events/{conversation_id}`.
 */
export function useCreateDeviceConversation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: Omit<CreateDeviceConversationRequest, "platform">,
    ): Promise<ChannelConversationItem> => {
      return api.post<ChannelConversationItem>("/channel-conversations", {
        platform: "device",
        ...data,
      } satisfies CreateDeviceConversationRequest);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: CHANNEL_CONVERSATIONS_ROOT,
      });
    },
  });
}

export function useUpdateChannelConversation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      id,
      ...data
    }: { readonly id: string } & UpdateChannelConversationRequest): Promise<ChannelConversationItem> => {
      return api.put<ChannelConversationItem>(
        `/channel-conversations/${id}`,
        data,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: CHANNEL_CONVERSATIONS_ROOT,
      });
    },
  });
}

export function useDeleteChannelConversation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      return api.delete<void>(`/channel-conversations/${id}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: CHANNEL_CONVERSATIONS_ROOT,
      });
      void queryClient.invalidateQueries({ queryKey: ["channel-bots"] });
    },
  });
}
