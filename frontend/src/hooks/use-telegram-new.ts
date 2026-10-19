import { useChannelConnectLinkContext } from "@/hooks/use-channel-connect-link";
import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  telegramNewBeginSchema,
  telegramNewConfigSchema,
  telegramNewLaunchSchema,
  telegramNewRequestSchema,
  type TelegramNewRequest,
} from "@/schemas/telegram-new";

const ROOT = "/channel-bots/telegram-new";

export function useTelegramNewConfiguration(requestId?: string, poll = true) {
  const link = useChannelConnectLinkContext();
  const actor = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: ["telegram-new", actor, requestId, link?.id],
    enabled: Boolean(actor),
    retry: false,
    staleTime: 0,
    refetchInterval: (query) => {
      const status = query.state.data?.request?.status;
      return poll &&
        status &&
        !["connected", "cancelled", "expired", "suspended"].includes(status)
        ? 2000
        : false;
    },
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
    queryFn: async () => {
      const config = telegramNewConfigSchema.parse(await api.get(requestId ? `${ROOT}?request_id=${encodeURIComponent(requestId)}` : ROOT));
      return link && !requestId ? { ...config, request: null } : config;
    },
  });
}

export function useTelegramNew(requestId?: string) {
  const link = useChannelConnectLinkContext();
  const actor = useAuthStore((state) => state.user?.id);
  const client = useQueryClient();
  const key = ["telegram-new", actor] as const;
  const configuration = useTelegramNewConfiguration(requestId);
  const connectedBotId =
    configuration.data?.request?.status === "connected"
      ? configuration.data.request.channel_bot_id
      : null;
  useEffect(() => {
    if (connectedBotId)
      void client.invalidateQueries({ queryKey: ["channel-bots"] });
  }, [client, connectedBotId]);
  const refresh = async () => {
    await client.invalidateQueries({ queryKey: key });
    await link?.refresh();
  };
  const begin = useMutation({
    mutationKey: key,
    gcTime: 0,
    mutationFn: async (input: {
      label: string;
      target_org_id?: string;
      auto_connect?: boolean;
    }) =>
      telegramNewLaunchSchema.parse(
        await api.post(link ? "/channel-connect-links/telegram/start" : ROOT, link ? { token: link.token } : telegramNewBeginSchema.parse(input)),
      ),
    onSettled: refresh,
  });
  const launch = useMutation({
    mutationKey: key,
    gcTime: 0,
    mutationFn: async (id: string) =>
      telegramNewLaunchSchema.parse(
        await api.post(`${ROOT}/requests/${id}/launch`),
      ),
    onSettled: refresh,
  });
  const cancel = useMutation({
    mutationKey: key,
    mutationFn: (id: string) => api.delete(`${ROOT}/requests/${id}`),
    onSettled: refresh,
  });
  const connect = useMutation({
    mutationKey: key,
    mutationFn: async (request: TelegramNewRequest) =>
      telegramNewRequestSchema.parse(
        await api.post(`${ROOT}/requests/${request.id}/connect`, {
          telegram_bot_id: request.telegram_bot_id,
          revision: request.revision,
        }),
      ),
    onSettled: async () => {
      await refresh();
      await client.invalidateQueries({ queryKey: ["channel-bots"] });
    },
  });
  return { configuration, begin, launch, cancel, connect };
}
