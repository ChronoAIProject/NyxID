import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  adminUsageResponseSchema,
  usageRangeError,
} from "@/schemas/admin-usage";
import type { AnalyticsFilters } from "@/schemas/usage-analytics";
import type { AdminUsageSearch } from "@/types/admin";

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
export function useAdminUsage(params: AdminUsageParams, enabled = true) {
  return useQuery({
    queryKey: ["admin", "usage", params],
    queryFn: async () =>
      adminUsageResponseSchema.parse(
        await api.get<unknown>(adminUsagePath(params)),
      ),
    enabled:
      enabled &&
      (params.period !== "custom" || !usageRangeError(params.from, params.to)),
    staleTime: 30_000,
    retry: false,
  });
}
