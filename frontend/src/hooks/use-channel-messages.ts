import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { ChannelMessageListResponse } from "@/types/channels";

export function useChannelMessages(
  conversationId: string,
  page: number,
  perPage: number = 50,
) {
  return useQuery({
    queryKey: ["channel-messages", conversationId, page, perPage],
    queryFn: () =>
      api.get<ChannelMessageListResponse>(
        `/channel-conversations/${conversationId}/messages?page=${String(page)}&per_page=${String(perPage)}`,
      ),
    enabled: Boolean(conversationId),
  });
}
