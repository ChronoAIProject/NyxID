import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { groupsApi } from "../../lib/api/groupsApi";
import { collaborationApi } from "../../lib/api/collaborationApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { groupKeys, useGroupAvailability } from "./useGroupRuntime";

/** New controls share the existing account, foreground and connectivity fence. */
export function useCollaborationSession(
  requiredFeature?: string,
  requireCapability = false,
) {
  const live = useGroupAvailability();
  const client = useQueryClient();
  const mounted = useRef(true);
  const lock = useRef(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{
    generation: number;
    message: string;
  }>();
  const activation = useRef({
    active: live.active,
    epoch: live.epoch,
    at: Date.now(),
    generation: 0,
  });
  if (
    activation.current.active !== live.active ||
    activation.current.epoch !== live.epoch
  )
    activation.current = {
      active: live.active,
      epoch: live.epoch,
      at: Date.now(),
      generation: activation.current.generation + 1,
    };
  const key = ["workspace", "collaboration", live.epoch] as const;
  const identity = useQuery({
    queryKey: [...key, "identity"],
    queryFn: ({ signal }) => groupsApi.identity(signal),
    enabled: live.active,
    retry: false,
    staleTime: 0,
  });
  const readable =
    live.active &&
    !identity.isError &&
    !identity.isFetching &&
    identity.dataUpdatedAt >= activation.current.at;
  const enabled =
    readable &&
    identity.data?.enabled_features.includes("assistant:shared-agents") ===
      true;
  const capability = useQuery({
    queryKey: [...key, "available"],
    queryFn: ({ signal }) => collaborationApi.available(signal),
    enabled: live.active && enabled,
    retry: false,
    staleTime: 0,
  });
  useEffect(() => {
    mounted.current = true;
    if (live.active) void client.invalidateQueries({ queryKey: key });
    else void client.cancelQueries({ queryKey: key });
    return () => {
      mounted.current = false;
    };
  }, [live.active, live.epoch, client]);
  useEffect(() => {
    setFailure(undefined);
  }, [live.active, live.epoch]);
  const generation = activation.current.generation;
  const clearError = () => setFailure(undefined);
  const valid = () =>
    mounted.current &&
    activation.current.generation === generation &&
    live.epoch === getAuthEpoch() &&
    live.available();
  const guard = () => {
    if (!valid())
      throw new Error(
        "This view is inactive or the account changed. Reopen it before continuing.",
      );
  };
  async function run<T>(
    action: () => Promise<T>,
    onSuccess?: (result: T) => void,
    authority?: { feature?: string | null; capability?: boolean },
  ) {
    if (lock.current || !valid()) return;
    lock.current = true;
    setBusy(true);
    clearError();
    try {
      const actor = await groupsApi.identity();
      if (!valid())
        throw new Error("Keep this view open and connected to continue.");
      // null explicitly drops only the feature admission gate for revocation.
      // Omitted/undefined keeps the caller's default gate in place.
      const feature =
        authority?.feature === undefined ? requiredFeature : authority.feature;
      if (feature && !actor.enabled_features.includes(feature))
        throw new Error(
          "This feature is no longer enabled for your account. Reload to review availability.",
        );
      if (authority?.capability ?? requireCapability) {
        const ready = await collaborationApi.available();
        if (!valid() || !ready)
          throw new Error(
            "Collaboration is unavailable until the server rollout is ready.",
          );
      }
      const result = await action();
      if (valid()) {
        await Promise.all([
          client.invalidateQueries({ queryKey: key }),
          client.invalidateQueries({ queryKey: groupKeys(live.epoch) }),
          client.invalidateQueries({ queryKey: ["workspace", "agents"] }),
        ]);
        if (valid()) onSuccess?.(result);
      }
      return result;
    } catch (cause) {
      if (valid())
        setFailure({
          generation,
          message:
            cause instanceof Error
              ? cause.message
              : "Could not save. Reload and try again.",
        });
      if (valid()) void client.invalidateQueries({ queryKey: key });
      return undefined;
    } finally {
      lock.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  return {
    ...live,
    key,
    identity,
    readable,
    activationAt: activation.current.at,
    enabled,
    capability,
    client,
    valid,
    guard,
    busy,
    error: failure?.generation === generation ? failure.message : undefined,
    clearError,
    run,
  };
}
