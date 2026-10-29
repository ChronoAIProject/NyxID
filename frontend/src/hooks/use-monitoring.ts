import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
  const identity = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const queryKey = ["monitoring", serviceId, identity] as const;
  const path = `/keys/${encodeURIComponent(serviceId)}/monitoring`;
  const authorityGuard = () => {
    if (!identity || useAuthStore.getState().user?.id !== identity)
      throw new Error("Account changed. Reload monitoring before continuing.");
  };
  const query = useQuery({
    queryKey,
    enabled: Boolean(identity && serviceId),
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
    onSuccess: (_result, input) => {
      if (input.kind === "save" || input.kind === "clear")
        void queryClient.invalidateQueries({ queryKey, exact: true });
    },
  });
  return { query, operation };
}
