import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  ActivityCallbackSupport,
  ChannelActivityResponse,
} from "@/types/channels";

export function useChannelActivities(
  scope: "bot" | "route",
  id: string,
  enabled: boolean,
  kind = "",
  page = 1,
) {
  const resource = scope === "bot" ? "channel-bots" : "channel-conversations";
  return useQuery({
    queryKey: ["channel-activities", scope, id, kind, page],
    queryFn: () =>
      api.get<ChannelActivityResponse>(
        `/${resource}/${id}/activities?per_page=20&page=${page}${kind ? `&kind=${encodeURIComponent(kind)}` : ""}`,
      ),
    enabled: enabled && !!id,
    refetchInterval: 15_000,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
  });
}

export function useActivityCallback(id: string) {
  return useQuery({
    queryKey: ["channel-activity-callback", id],
    queryFn: () =>
      api.get<ActivityCallbackSupport>(
        `/channel-conversations/${id}/activity-callback`,
      ),
    enabled: !!id,
    refetchInterval: 15_000,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
  });
}

export function useSetActivityCallback(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (enabled: boolean) =>
      api.put<void>(`/channel-conversations/${id}/activity-callback`, {
        enabled,
      }),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["channel-activity-callback", id] }),
  });
}
