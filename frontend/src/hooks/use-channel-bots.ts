import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  ChannelBotListResponse,
  ChannelBotDetail,
  ChannelBotItem,
  CreateChannelBotRequest,
  CreateChannelBotResponse,
  UpdateChannelBotRequest,
} from "@/types/channels";

// ─────────────────────────────────────────────────────────────────────────────
// Query keys
// ─────────────────────────────────────────────────────────────────────────────
//
// Bots are scoped either to the caller's personal space or to an org. We
// encode that in the query key so React Query caches personal and per-org
// results independently; switching the scope selector must not return stale
// data from the previous scope.

const CHANNEL_BOTS_ROOT = ["channel-bots"] as const;

export const channelBotsQueryKeys = {
  all: CHANNEL_BOTS_ROOT,
  list: (orgId: string | null) =>
    [...CHANNEL_BOTS_ROOT, "list", orgId ?? "personal"] as const,
  detail: (id: string) => [...CHANNEL_BOTS_ROOT, "detail", id] as const,
} as const;

// ─────────────────────────────────────────────────────────────────────────────
// Queries
// ─────────────────────────────────────────────────────────────────────────────

interface UseChannelBotsParams {
  /** `null` or omitted lists personal bots. When set, lists bots owned by the
   *  given org (caller must be admin of the org). */
  readonly orgId?: string | null;
}

export function useChannelBots(params: UseChannelBotsParams = {}) {
  const orgId = params.orgId ?? null;
  return useQuery({
    queryKey: channelBotsQueryKeys.list(orgId),
    queryFn: async (): Promise<readonly ChannelBotItem[]> => {
      const path = orgId
        ? `/channel-bots?org_id=${encodeURIComponent(orgId)}`
        : "/channel-bots";
      const res = await api.get<ChannelBotListResponse>(path);
      return res.bots;
    },
  });
}

export function useChannelBot(id: string) {
  return useQuery({
    queryKey: channelBotsQueryKeys.detail(id),
    queryFn: async (): Promise<ChannelBotDetail> => {
      return api.get<ChannelBotDetail>(`/channel-bots/${id}`);
    },
    enabled: Boolean(id),
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Mutations
// ─────────────────────────────────────────────────────────────────────────────

export function useCreateChannelBot() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateChannelBotRequest,
    ): Promise<CreateChannelBotResponse> => {
      return api.post<CreateChannelBotResponse>("/channel-bots", data);
    },
    onSuccess: () => {
      // Invalidate every list regardless of scope -- the bot could have
      // landed in any scope the user is viewing.
      void queryClient.invalidateQueries({ queryKey: CHANNEL_BOTS_ROOT });
    },
  });
}

export function useDeleteChannelBot() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      return api.delete<void>(`/channel-bots/${id}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: CHANNEL_BOTS_ROOT });
    },
  });
}

export function useUpdateChannelBot() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      id,
      data,
    }: {
      readonly id: string;
      readonly data: UpdateChannelBotRequest;
    }): Promise<ChannelBotDetail> => {
      return api.patch<ChannelBotDetail>(`/channel-bots/${id}`, data);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: channelBotsQueryKeys.detail(variables.id),
      });
      void queryClient.invalidateQueries({ queryKey: CHANNEL_BOTS_ROOT });
    },
  });
}

export function useVerifyChannelBot() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      return api.post<void>(`/channel-bots/${id}/verify`);
    },
    onSuccess: (_data, id) => {
      void queryClient.invalidateQueries({
        queryKey: channelBotsQueryKeys.detail(id),
      });
      void queryClient.invalidateQueries({ queryKey: CHANNEL_BOTS_ROOT });
    },
  });
}
