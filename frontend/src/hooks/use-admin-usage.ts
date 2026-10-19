import { useEffect } from "react";
import { queryOptions, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  adminUsageResponseSchema,
  usageRangeError,
} from "@/schemas/admin-usage";
import type { AnalyticsFilters } from "@/schemas/usage-analytics";
import type { AdminUsageSearch } from "@/types/admin";
import type { AdminUsageResponse } from "@/types/admin";
import { useAuthStore } from "@/stores/auth-store";

export type AdminUsageParams = AdminUsageSearch &
  Partial<Pick<AnalyticsFilters, "services" | "actors" | "owners">>;

export function adminUsagePath(params: AdminUsageParams): string {
  const query = new URLSearchParams();
  if (params.period === "custom") {
    if (params.from) query.set("from", params.from);
    if (params.to) query.set("to", params.to);
  } else {
    query.set("period", params.period);
  }
  if (params.user) query.set("user", params.user);
  if (params.service) query.set("service", params.service);
  for (const key of ["services", "actors", "owners"] as const) {
    if (params[key]?.length)
      query.set(key, [...new Set(params[key])].sort().join(","));
  }
  query.set("sort", params.sort);
  query.set("metric", params.metric);
  query.set("page", String(params.page));
  query.set("per_page", String(params.per_page));
  return `/admin/usage?${query.toString()}`;
}
export function adminUsageQueryOptions(
  params: AdminUsageParams,
  userId?: string,
) {
  const path = adminUsagePath(params);
  return queryOptions({
    queryKey: ["admin", "usage", userId, path],
    queryFn: async () =>
      adminUsageResponseSchema.parse(await api.get<unknown>(path)),
    staleTime: 5 * 60_000,
    gcTime: 10 * 60_000,
    retry: false,
  });
}

export function useAdminUsage(params: AdminUsageParams, enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    ...adminUsageQueryOptions(params, userId),
    enabled:
      enabled &&
      (params.period !== "custom" || !usageRangeError(params.from, params.to)),
  });
}

export function usePreloadAdminUsageDetails(
  data: AdminUsageResponse | undefined,
  search: AdminUsageParams,
) {
  const client = useQueryClient();
  const userId = useAuthStore((state) => state.user?.id);
  const path = adminUsagePath(search);
  useEffect(() => {
    if (!data || !userId) return;
    const users = [...new Set(data.ranking.map((row) => row.user.id))];
    let next = 0;
    let active = true;
    const preload = async () => {
      while (active && next < users.length) {
        const user = users[next++];
        await client.prefetchQuery(
          adminUsageQueryOptions({ ...search, user }, userId),
        );
      }
    };
    // Keep the reporting API responsive while warming every visible user's detail.
    for (let i = 0; i < Math.min(3, users.length); i++) void preload();
    return () => {
      active = false;
    };
  }, [client, data, path, search, userId]);
}
