import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { remoteMachineUsageSchema } from "@/schemas/remote-machine-usage";

export function useRemoteMachineUsage(machineId: string) {
  const owner = useAuthStore((state) => state.user?.id);
  const query = useQuery({
    queryKey: ["remote-machine-usage", owner, machineId],
    queryFn: async () =>
      remoteMachineUsageSchema.parse(
        await api.get<unknown>(
          `/remote-machines/${encodeURIComponent(machineId)}/usage`,
          {
            authorityGuard: () => {
              if (!owner || useAuthStore.getState().user?.id !== owner)
                throw new Error("Your account changed. Reload Machine usage.");
            },
          },
        ),
      ),
    enabled: Boolean(owner && machineId),
    refetchInterval: 10000,
    retry: false,
  });
  return { ...query, data: query.isError ? undefined : query.data };
}
