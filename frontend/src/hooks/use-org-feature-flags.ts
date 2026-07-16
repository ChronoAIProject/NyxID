import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { orgsQueryKeys } from "@/hooks/use-orgs";
import type {
  FeatureFlagListResponse,
  FlagTargetKind,
  SetFeatureFlagRequest,
} from "@/schemas/feature-flags";

const ROOT = ["org-feature-flags"] as const;

export const orgFeatureFlagKeys = {
  detail: (orgId: string) => [...ROOT, orgId] as const,
} as const;

/**
 * Admin view of every code-declared flag plus this org's overrides.
 * Admin-gated on the backend (`require_org_admin`).
 */
export function useOrgFeatureFlags(orgId: string) {
  return useQuery({
    queryKey: orgFeatureFlagKeys.detail(orgId),
    queryFn: (): Promise<FeatureFlagListResponse> =>
      api.get<FeatureFlagListResponse>(`/orgs/${orgId}/feature-flags`),
    enabled: Boolean(orgId),
  });
}

function invalidate(
  queryClient: ReturnType<typeof useQueryClient>,
  orgId: string,
) {
  void queryClient.invalidateQueries({
    queryKey: orgFeatureFlagKeys.detail(orgId),
  });
  // Refresh the resolved `enabled_features` so gated UI updates live.
  void queryClient.invalidateQueries({
    queryKey: orgsQueryKeys.detail(orgId),
  });
}

/** Upsert one override (org / role / user scope) for a flag. */
export function useSetOrgFeatureFlag(orgId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      flagKey,
      body,
    }: {
      flagKey: string;
      body: SetFeatureFlagRequest;
    }): Promise<unknown> =>
      api.put(`/orgs/${orgId}/feature-flags/${flagKey}`, body),
    onSuccess: () => invalidate(queryClient, orgId),
  });
}

/** Clear one override, reverting that scope to the next-broader layer. */
export function useClearOrgFeatureFlag(orgId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      flagKey,
      targetKind,
      targetValue,
    }: {
      flagKey: string;
      targetKind: FlagTargetKind;
      targetValue?: string | null;
    }): Promise<void> => {
      const params = new URLSearchParams({ target_kind: targetKind });
      if (targetValue != null) params.set("target_value", targetValue);
      return api.delete<void>(
        `/orgs/${orgId}/feature-flags/${flagKey}?${params.toString()}`,
      );
    },
    onSuccess: () => invalidate(queryClient, orgId),
  });
}
