import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { billingUsagePath } from "@/hooks/use-billing";
import { billingUsageResponseSchema } from "@/schemas/billing";
import { useAuthStore } from "@/stores/auth-store";

export function useRemoteMachineServiceUsage(
  machineId: string,
  enabled: boolean,
) {
  const owner = useAuthStore((state) => state.user?.id);
  const query = useQuery({
    queryKey: ["remote-machine-service-usage", owner, machineId, "30d"],
    queryFn: async () => {
      const response = billingUsageResponseSchema.parse(
        await api.get(
          `${billingUsagePath("30d")}&machine_id=${encodeURIComponent(machineId)}`,
          {
            authorityGuard: () => {
              if (!owner || useAuthStore.getState().user?.id !== owner)
                throw new Error("Your account changed. Reload service usage.");
            },
          },
        ),
      );
      if (
        response.owner_id !== owner ||
        response.period !== "30d" ||
        response.machine_id !== machineId
      )
        throw new Error(
          "The service usage response does not match this account and period.",
        );
      return response.rows.filter(
        (row) => row.machine_usage?.node_id === machineId,
      );
    },
    enabled: enabled && Boolean(owner && machineId),
    retry: false,
    refetchOnWindowFocus: false,
  });
  return { ...query, data: query.isError ? undefined : query.data };
}
