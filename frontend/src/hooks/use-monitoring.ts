import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG, userHasFeature } from "@/lib/feature-flags";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import type {
  MonitoringConfig,
  MonitoringResponse,
  MonitoringResult,
} from "@/schemas/monitoring";

type Operation =
  | { kind: "save"; revision: number; checks: MonitoringConfig["owner_config"] }
  | { kind: "clear"; revision: number }
  | { kind: "validate"; checks: MonitoringConfig["owner_config"] }
  | { kind: "probe" };

export function useMonitoring(serviceId: string) {
  const enabled = useFeature(FEATURE_FLAG.SERVICE_MONITORING);
  const identity = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const queryKey = ["monitoring", serviceId, identity] as const;
  const path = `/keys/${encodeURIComponent(serviceId)}/monitoring`;
  const authorityGuard = () => {
    const current = useAuthStore.getState().user;
    if (!enabled || !userHasFeature(current, FEATURE_FLAG.SERVICE_MONITORING))
      throw new Error("Monitoring is not enabled.");
    if (!identity || current?.id !== identity)
      throw new Error("Account changed. Reload monitoring before continuing.");
  };
  const query = useQuery({
    queryKey,
    enabled: Boolean(enabled && identity && serviceId),
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    queryFn: async ({ signal }) => {
      authorityGuard();
      const response = await apiClient<MonitoringResponse>(path, {
        signal,
        authorityGuard,
      });
      authorityGuard();
      return response;
    },
  });
  const operation = useMutation({
    retry: false,
    gcTime: 0,
    mutationFn: async (input: Operation) => {
      authorityGuard();
      const options = { authorityGuard };
      let result: MonitoringConfig | MonitoringResult;
      switch (input.kind) {
        case "save":
          result = await apiClient<MonitoringConfig>(path, {
            ...options,
            method: "PUT",
            body: { revision: input.revision, ...input.checks },
          });
          break;
        case "clear":
          result = await apiClient<MonitoringConfig>(
            `${path}?revision=${input.revision}`,
            { ...options, method: "DELETE" },
          );
          break;
        case "validate":
          result = await apiClient<MonitoringResult>(`${path}/validate`, {
            ...options,
            method: "POST",
            body: input.checks,
          });
          break;
        case "probe":
          result = await apiClient<MonitoringResult>(`${path}/probe`, {
            ...options,
            method: "POST",
          });
          break;
      }
      authorityGuard();
      return result;
    },
    onSuccess: async (_result, input) => {
      if (input.kind !== "validate")
        await queryClient.invalidateQueries({ queryKey, exact: true });
    },
  });
  return { query, operation };
}
