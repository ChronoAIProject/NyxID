import { createContext, useContext } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { channelConnectLinkSchema, type ChannelConnectLink } from "@/schemas/channel-connect-links";

export const ChannelConnectLinkContext = createContext<{
  token: string;
  id: string;
  telegramRequestId?: string;
  connectionId?: string;
  recoveryUnavailable?: boolean;
  isRecoveryCurrent?: (requestId: string, observedHostedGeneration: number | undefined) => boolean;
  getRecoveryGeneration?: () => number;
  refresh: () => Promise<string | null | void>;
} | null>(null);

export const useChannelConnectLinkContext = () =>
  useContext(ChannelConnectLinkContext);

export function useChannelConnectLink(token: string, actor?: string) {
  const client = useQueryClient();
  const preview = useQuery({
    queryKey: ["channel-connect-preview", token],
    queryFn: async () =>
      channelConnectLinkSchema.parse(
        await api.post("/channel-connect-links/preview", { token }),
      ),
    retry: false,
    gcTime: 0,
  });
  const key = ["channel-connect-link", actor, preview.data?.id];
  const status = useQuery({
    queryKey: key,
    enabled: Boolean(actor && preview.data?.id),
    queryFn: async () =>
      channelConnectLinkSchema.parse(
        await api.get(`/channel-connect-links/${preview.data!.id}`),
      ),
    refetchInterval: (query) =>
      query.state.data?.status === "pending" ? 2000 : false,
    refetchIntervalInBackground: false,
    retry: false,
    gcTime: 0,
  });
  const refresh = async () => {
    await client.invalidateQueries({ queryKey: ["channel-connect-link"] });
    const current = client.getQueryState<ChannelConnectLink>(key);
    if (current?.status !== "success" || current.isInvalidated || current.data?.status !== "pending") {
      return null;
    }
    return current.data.connection_id;
  };
  const isRecoveryCurrent = (requestId: string, observedHostedGeneration: number | undefined) => {
    const current = client.getQueryState<ChannelConnectLink>(key);
    // A hosted status failure also invalidates a previously successful Telegram
    // read. Both queries must recover before another form can be issued or used.
    return current?.status === "success" && !current.isInvalidated
      && current.data?.status === "pending" && !current.data.telegram_requires_original_actor
      && current.data.telegram_request_id === requestId
      && observedHostedGeneration === current.errorUpdateCount;
  };
  const action = useMutation({
    mutationFn: async (name: "decline" | "retry") =>
      channelConnectLinkSchema.parse(
        await api.post(`/channel-connect-links/${name}`, { token }),
      ),
    onSuccess: (data) => client.setQueryData(key, data),
  });
  return { preview, status, action, refresh, isRecoveryCurrent,
    getRecoveryGeneration: () => client.getQueryState(key)?.errorUpdateCount ?? 0,
    recoveryUnavailable: !status.isSuccess || status.data.status !== "pending" || Boolean(status.data.telegram_requires_original_actor),
  };
}
