import { useState } from "react";

/**
 * Pull to refresh shows its spinner only for the pull itself. Background
 * polling, refocus and mutation refetches never flash it; bind
 * `refreshing` to this, never to a query's `isFetching`/`isRefetching`.
 */
export function usePullRefresh(refetch: () => Promise<unknown> | void) {
  const [refreshing, setRefreshing] = useState(false);
  return {
    refreshing,
    onRefresh: () => {
      setRefreshing(true);
      void Promise.resolve(refetch()).finally(() => setRefreshing(false));
    },
  };
}
