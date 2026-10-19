import { createContext, useContext } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { channelConnectLinkSchema } from "@/schemas/channel-connect-links";

export const ChannelConnectLinkContext = createContext<{
  token: string;
  id: string;
  telegramRequestId?: string;
  connectionId?: string;
  refresh: () => Promise<unknown>;
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
  const refresh = () =>
    client.invalidateQueries({ queryKey: ["channel-connect-link"] });
  const action = useMutation({
    mutationFn: async (name: "decline" | "retry") =>
      channelConnectLinkSchema.parse(
        await api.post(`/channel-connect-links/${name}`, { token }),
      ),
    onSuccess: (data) => client.setQueryData(key, data),
  });
  return { preview, status, action, refresh };
}
