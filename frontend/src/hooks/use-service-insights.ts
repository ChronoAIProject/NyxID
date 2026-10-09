import { useQuery } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  serviceInsightsResponseSchema,
  type ServiceInsight,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";
import { loadConfiguredServiceInsights } from "@/lib/service-insights-compat";

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
  // Compatibility billing and key access depend on the selected credential and
  // owner, which can change while the connection id stays the same.
  const inputs = connections
    .map((connection) => ({
      id: connection.id,
      api_key_id: connection.api_key_id,
      credential_binding: connection.credential_binding,
      credential_source: connection.credential_source,
      credential_type: connection.credential_type,
      credential_missing: connection.credential_missing,
      oauth_app_source: connection.oauth_app_source,
      has_own_oauth_app: Boolean(connection.oauth_client_id?.trim()),
      connection_id: connection.connection_id,
      // Healthy OAuth rows prove their app, so reconnecting changes the label.
      status: connection.status,
      connection_status: connection.connection_status,
      auth_method: connection.auth_method,
      catalog_service_id: connection.catalog_service_id,
      catalog_service_slug: connection.catalog_service_slug,
      node_id: connection.node_id,
      has_node_binding: connection.has_node_binding,
      is_active: connection.is_active,
      auto_connected: connection.auto_connected,
      byok_pricing: connection.byok_pricing,
      platform_key_pricing: connection.platform_key_pricing,
    }))
    .sort((a, b) => a.id.localeCompare(b.id));
  const query = useQuery({
    queryKey: ["keys", "insights", identity, inputs, apiKeyId ?? null],
    enabled: !supplied && !!identity && ids.length > 0,
    queryFn: async () => {
      const batches = Array.from(
        { length: Math.ceil(ids.length / 100) },
        (_, i) => ids.slice(i * 100, (i + 1) * 100),
      );
      try {
        const results = await Promise.allSettled(
          batches.map(async (batch) => {
            const response = await api.get<unknown>(
              `/service-insights?ids=${encodeURIComponent(batch.join(","))}${apiKeyId ? `&api_key_id=${encodeURIComponent(apiKeyId)}` : ""}`,
            );
            return serviceInsightsResponseSchema.parse(response).connections;
          }),
        );
        const failures = results.flatMap((result) =>
          result.status === "rejected" ? [result.reason as unknown] : [],
        );
        const blockingFailure = failures.find(
          (error) =>
            !(
              error instanceof ApiError &&
              [404, 405, 501].includes(error.status)
            ),
        );
        if (failures.length) throw blockingFailure ?? failures[0];
        return results.flatMap((result) =>
          result.status === "fulfilled" ? result.value : [],
        );
      } catch (error) {
        if (
          !apiKeyId &&
          error instanceof ApiError &&
          [404, 405, 501].includes(error.status)
        ) {
          return loadConfiguredServiceInsights(connections, identity!);
        }
        throw error;
      }
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
