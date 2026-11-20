import { useCallback } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { modeAQueryIdentity } from "@/components/cli-wizard/client";
import { useAuthStore } from "@/stores/auth-store";
import type { KeyInfo } from "@/types/keys";

// Assistant-only: keep use-keys.ts and the CLI wizard source closure unchanged.
// The drift-guard test checks this mirrored list key against the real useKeys.
export function useCurrentKeysReader() {
  const client = useQueryClient();
  return useCallback(() => {
    const authority = modeAQueryIdentity() ?? useAuthStore.getState().user?.id;
    const query = client.getQueryState<readonly KeyInfo[]>(["keys", "list", authority]);
    const isError = query?.status === "error";
    const fetchStatus = query?.fetchStatus;
    return {
      authority,
      data: isError ? undefined : query?.data,
      isError,
      isRefreshing: fetchStatus !== undefined && fetchStatus !== "idle",
      fetchStatus,
    };
  }, [client]);
}
