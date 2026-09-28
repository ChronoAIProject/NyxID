import { useQuery } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  serviceInsightsResponseSchema,
  type ServiceInsight,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";

export interface ServiceInsightsState {
  readonly connections: ReadonlyMap<string, ServiceInsight>;
  readonly status: "loading" | "ready" | "unavailable" | "restricted" | "error";
  readonly refresh: () => void;
}

export function useServiceInsights(
  connections: readonly KeyInfo[],
  supplied?: ServiceInsightsState,
  apiKeyId?: string,
): ServiceInsightsState {
  const identity = useAuthStore((state) => state.user?.id);
  const ids = [
    ...new Set(connections.map((connection) => connection.id)),
  ].sort();
  const query = useQuery({
    queryKey: ["keys", "insights", identity, ids, apiKeyId ?? null],
    enabled: !supplied && !!identity && ids.length > 0,
    queryFn: async () => {
      const batches = Array.from(
        { length: Math.ceil(ids.length / 100) },
        (_, i) => ids.slice(i * 100, (i + 1) * 100),
      );
      const results = await Promise.all(
        batches.map(async (batch) => {
          const response = await api.get<unknown>(
            `/service-insights?ids=${encodeURIComponent(batch.join(","))}${apiKeyId ? `&api_key_id=${encodeURIComponent(apiKeyId)}` : ""}`,
          );
          return serviceInsightsResponseSchema.parse(response).connections;
        }),
      );
      return results.flat();
    },
    retry: false,
    staleTime: 30_000,
    refetchOnWindowFocus: true,
  });
  if (supplied) return supplied;
  const unavailable =
    query.error instanceof ApiError &&
    [404, 405, 501].includes(query.error.status);
  return {
    // Never retain privileged summaries after an authorization/network failure.
    connections: new Map(
      query.isError
        ? []
        : (query.data ?? []).map((item) => [item.service_id, item]),
    ),
    status:
      query.error instanceof ApiError && query.error.status === 403
        ? "restricted"
        : query.isError
          ? unavailable
            ? "unavailable"
            : "error"
          : query.isPending && ids.length
            ? "loading"
            : "ready",
    refresh: () => {
      void query.refetch();
    },
  };
}
