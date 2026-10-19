import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { channelPlatformViews, platformView } from "@/lib/channel-platforms";
import type { ChannelPlatformsResponse } from "@/types/channels";

export const channelPlatformsQueryKey = ["channel-platforms"] as const;

export function useChannelPlatforms() {
  return useQuery({
    queryKey: channelPlatformsQueryKey,
    queryFn: () => api.get<ChannelPlatformsResponse>("/channel-platforms"),
    staleTime: 60_000,
  });
}

/** Presentation of API descriptors; absent catalog entries never enable creation. */
export function useChannelPlatformViews() {
  const query = useChannelPlatforms();
  const platforms = channelPlatformViews(query.data?.platforms ?? []);
  return { ...query, platforms, getPlatform: (id: string) => platforms[id] ?? platformView(undefined, id) };
}
