import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  AdminFeatureFlagListResponse,
  AdminFeatureFlagTargetKind,
  SetAdminFeatureFlagRequest,
} from "@/schemas/admin-feature-flags";

const QUERY_KEY = ["admin", "feature-flags"] as const;

export function useAdminFeatureFlags() {
  return useQuery({
    queryKey: QUERY_KEY,
    queryFn: (): Promise<AdminFeatureFlagListResponse> =>
      api.get<AdminFeatureFlagListResponse>("/admin/feature-flags"),
  });
}

export function useSetAdminFeatureFlag() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      flagKey,
      body,
    }: {
      flagKey: string;
      body: SetAdminFeatureFlagRequest;
    }): Promise<unknown> =>
      api.put(`/admin/feature-flags/${encodeURIComponent(flagKey)}`, body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: ["orgs"] });
      void queryClient.invalidateQueries({ queryKey: ["user"] });
    },
  });
}

export function useClearAdminFeatureFlag() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      flagKey,
      targetKind,
      targetKey,
    }: {
      flagKey: string;
      targetKind: AdminFeatureFlagTargetKind;
      targetKey?: string | null;
    }): Promise<void> => {
      const params = new URLSearchParams({ target_kind: targetKind });
      if (targetKey != null) params.set("target_key", targetKey);
      return api.delete<void>(
        `/admin/feature-flags/${encodeURIComponent(flagKey)}?${params.toString()}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: ["orgs"] });
      void queryClient.invalidateQueries({ queryKey: ["user"] });
    },
  });
}
