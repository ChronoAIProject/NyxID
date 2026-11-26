import { useEffect } from "react";
import { onlineManager, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantApi } from "../../lib/api/assistantApi";
import {
  composerSettingsRecoveryInterval,
  scheduleComposerSettingsRecovery,
} from "./composerSettings";

export function useComposerSettingsQuery(
  epoch: number,
  active: boolean,
  available: () => boolean,
) {
  const client = useQueryClient();
  const key = ["assistant", "composer-settings", epoch] as const;
  const settings = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => assistantApi.settings(signal),
    // Error recovery owns the next automatic attempt, even across reconnect or
    // foreground refetches. Explicit Retry can still refetch a disabled query.
    enabled: (query) => active && query.state.status !== "error",
    retry: false,
  });
  const { error, errorUpdatedAt, errorUpdateCount, fetchStatus, refetch, status } =
    settings;
  useEffect(() => {
    if (!active || fetchStatus !== "idle") return;
    const delay = composerSettingsRecoveryInterval({
      state: { error, errorUpdateCount, status },
    });
    if (delay === false) return;
    return scheduleComposerSettingsRecovery(errorUpdatedAt, delay, () => {
      const current = client.getQueryState(["assistant", "composer-settings", epoch]);
      if (
        available() && onlineManager.isOnline() &&
        current?.status === "error" &&
        current.fetchStatus === "idle" &&
        current.errorUpdatedAt === errorUpdatedAt &&
        current.errorUpdateCount === errorUpdateCount
      ) void refetch({ cancelRefetch: false });
    });
  }, [
    active, available, client, epoch, error, errorUpdatedAt, errorUpdateCount,
    fetchStatus, refetch, status,
  ]);
  return settings;
}
