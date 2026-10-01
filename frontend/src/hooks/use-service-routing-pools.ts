import { useQueries } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  servicePoolListResponseSchema,
  type ServicePool,
} from "@/schemas/pools";
import type { KeyInfo } from "@/types/keys";

export interface ServiceRoutingPools {
  pools: readonly ServicePool[];
  loading: boolean;
  incomplete: boolean;
}

export function useServiceRoutingPools(
  keys: readonly KeyInfo[],
  enabled = true,
): ServiceRoutingPools {
  const identity = useAuthStore((state) => state.user?.id);
  // Pool management reads currently require organization admin access.
  const orgIds = [
    ...new Set(
      keys.flatMap((key) => {
        const source = key.credential_source;
        return source?.type === "org" &&
          source.allowed &&
          source.role === "admin"
          ? [source.org_id]
          : [];
      }),
    ),
  ].sort();
  const queries = useQueries({
    queries: [undefined, ...orgIds].map((orgId) => ({
      queryKey: ["service-pools", "routing", identity, orgId],
      enabled: enabled && !!identity && keys.length > 0,
      queryFn: async () =>
        servicePoolListResponseSchema.parse(
          await api.get<unknown>(
            orgId
              ? `/service-pools?org_id=${encodeURIComponent(orgId)}`
              : "/service-pools",
          ),
        ).pools,
      retry: false,
      staleTime: 30_000,
    })),
  });
  return {
    pools: queries.flatMap((query) =>
      query.isError ? [] : (query.data ?? []),
    ),
    loading: queries.some((query) => query.isLoading),
    incomplete:
      queries.some((query) => query.isError) ||
      keys.some(
        (key) =>
          key.credential_source?.type === "org" &&
          (key.credential_source.role !== "admin" ||
            !key.credential_source.allowed),
      ),
  };
}
