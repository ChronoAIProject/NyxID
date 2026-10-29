import { useEffect, useMemo, useState } from "react";
import { AppState } from "react-native";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { assistantApi } from "../../lib/api/assistantApi";
import { groupsApi } from "../../lib/api/groupsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { useGroupsLive } from "../../features/groups/useGroupsLive";
import { groupKeys } from "../../features/groups/groupLiveRefresh";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import { workspaceRecents } from "./workspaceSidebarModel";

export function useWorkspaceRecents(visible: boolean, agentId?: string) {
  const epoch = getAuthEpoch();
  const { isConnected } = useNetworkStatus();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  const active = visible && foreground && isConnected;
  const live = useGroupsLive(
    epoch,
    active,
    () => active && epoch === getAuthEpoch(),
  );
  // Share the existing live invalidation prefix, not the agent-scoped query.
  const chats = useInfiniteQuery({
    queryKey: [
      "workspace",
      "drawer-chats",
      epoch,
      "recents",
      ...(agentId ? [agentId] : []),
    ],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) =>
      assistantApi.list(pageParam, agentId, signal),
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: active,
    refetchInterval: livePollInterval(5000, live),
    retry: false,
  });
  const crews = useQuery({
    queryKey: [...groupKeys(epoch), "list"],
    queryFn: ({ signal }) => groupsApi.list(signal),
    enabled: active,
    refetchInterval: livePollInterval(5000, live),
    retry: false,
  });
  const conversations = useMemo(
    () => chats.data?.pages.flatMap((page) => page.conversations) ?? [],
    [chats.data],
  );
  const rows = useMemo(
    () => workspaceRecents(conversations, agentId ? [] : (crews.data ?? [])),
    [conversations, crews.data, agentId],
  );
  return {
    rows,
    conversations,
    crews: crews.data ?? [],
    loading: active && (chats.isPending || crews.isPending),
    refreshing: chats.isRefetching || crews.isRefetching,
    online: isConnected,
    error: chats.isError || crews.isError,
    active,
    hasNextPage: chats.hasNextPage,
    loadingMore: chats.isFetchingNextPage,
    refresh: async () => {
      if (!active || epoch !== getAuthEpoch()) return;
      await Promise.allSettled([chats.refetch(), crews.refetch()]);
    },
    loadMore: () => {
      if (
        active &&
        epoch === getAuthEpoch() &&
        chats.hasNextPage &&
        !chats.isFetchingNextPage
      )
        void chats.fetchNextPage();
    },
  };
}
