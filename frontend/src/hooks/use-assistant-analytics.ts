import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { assistantAnalyticsSchema } from "@/schemas/assistant-analytics";
import { useAuthStore } from "@/stores/auth-store";

export function useAssistantAnalytics(
  from: string,
  to: string,
  enabled: boolean,
  threshold = 3,
) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: ["admin", "assistant-analytics", userId, from, to, threshold],
    queryFn: async () =>
      assistantAnalyticsSchema.parse(
        await api.get(
          `/admin/assistant/analytics?${new URLSearchParams({ from, to, threshold: String(threshold) })}`,
        ),
      ),
    enabled,
    retry: false,
    refetchInterval: 30_000,
  });
}
