import { useEffect } from "react";
import { AppState } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { agentsApi } from "../../lib/api/agentsApi";
import { groupsApi } from "../../lib/api/groupsApi";
import { groupKeys } from "../groups/groupLiveRefresh";
import type { QuickAccessAgent, QuickAccessCrew } from "./quickAccessModel";
import {
  quickAccessRetryDelay,
  refetchIfDue,
  retryQuickAccessRead,
} from "./quickAccessReadRetry";

export const QUICK_ACCESS_SOURCES_STALE_MS = 30_000;

/** The side menu's own cache keys, so the lists share data and refreshes. */
export const quickAccessSourceKeys = (epoch: number) => ({
  agents: ["workspace", "agents", "drawer", epoch] as const,
  crews: [...groupKeys(epoch), "list"] as const,
});

/**
 * The agents and crews quick access can pin, from the side menu's own
 * queries. Each loads only when asked for; `undefined` means not loaded (yet,
 * or after an error). On the folded Duo the rail may be the only reader (the
 * side menu polls only while open), so these recover like the list itself:
 * transient failures retry, and a failed or stale list is read again on
 * reconnect and when the app returns to the foreground.
 */
export function useQuickAccessSources({
  agents,
  crews,
}: {
  agents: boolean;
  crews: boolean;
}): {
  agents?: readonly QuickAccessAgent[];
  crews?: readonly QuickAccessCrew[];
} {
  const client = useQueryClient();
  const epoch = getAuthEpoch();
  const keys = quickAccessSourceKeys(epoch);
  const recovery = {
    staleTime: QUICK_ACCESS_SOURCES_STALE_MS,
    retry: (failureCount: number, error: unknown) =>
      epoch === getAuthEpoch() && retryQuickAccessRead(failureCount, error),
    retryDelay: quickAccessRetryDelay,
    refetchOnReconnect: true,
  };
  const agentList = useQuery({
    queryKey: keys.agents,
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: agents,
    ...recovery,
  });
  const crewList = useQuery({
    queryKey: keys.crews,
    queryFn: ({ signal }) => groupsApi.list(signal),
    enabled: crews,
    ...recovery,
  });
  useEffect(() => {
    if (!agents && !crews) return;
    const subscription = AppState.addEventListener("change", (state) => {
      if (state !== "active" || epoch !== getAuthEpoch()) return;
      const current = quickAccessSourceKeys(epoch);
      if (agents)
        refetchIfDue(client, current.agents, QUICK_ACCESS_SOURCES_STALE_MS);
      if (crews)
        refetchIfDue(client, current.crews, QUICK_ACCESS_SOURCES_STALE_MS);
    });
    return () => subscription.remove();
  }, [client, agents, crews, epoch]);
  return {
    agents: agents ? agentList.data?.agents : undefined,
    crews: crews ? crewList.data : undefined,
  };
}
