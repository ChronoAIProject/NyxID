import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { nyxbotModelUsageSchema } from "@/schemas/nyxbot-model-usage";
export type ModelFilter = { model?: string; unknown?: boolean };
export function nyxbotModelParams(
  from: string,
  to: string,
  filter: ModelFilter,
) {
  return new URLSearchParams({
    from,
    to,
    ...(filter.unknown
      ? { unknown_model: "true" }
      : filter.model
        ? { model: filter.model }
        : {}),
  });
}
export function useNyxbotModelUsage(
  from: string,
  to: string,
  filter: ModelFilter,
  enabled: boolean,
) {
  const userId = useAuthStore((state) => state.user?.id);
  const params = nyxbotModelParams(from, to, filter).toString();
  return useQuery({
    queryKey: ["admin", "nyxbot-model-usage", userId, params],
    queryFn: async () =>
      nyxbotModelUsageSchema.parse(
        await api.get(`/admin/assistant/model-usage?${params}`),
      ),
    enabled,
    retry: false,
    refetchInterval: 30_000,
  });
}
