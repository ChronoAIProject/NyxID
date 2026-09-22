import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  adminUsageResponseSchema,
  usageRangeError,
} from "@/schemas/admin-usage";
import type { AdminUsageSearch } from "@/types/admin";

export function adminUsagePath(params: AdminUsageSearch): string {
  const query = new URLSearchParams();
  if (params.period === "custom") {
    if (params.from) query.set("from", params.from);
    if (params.to) query.set("to", params.to);
  } else {
    query.set("period", params.period);
  }
  if (params.user) query.set("user", params.user);
  if (params.service) query.set("service", params.service);
  query.set("sort", params.sort);
  query.set("metric", params.metric);
  query.set("page", String(params.page));
  query.set("per_page", String(params.per_page));
  return `/admin/usage?${query.toString()}`;
}
export function useAdminUsage(params: AdminUsageSearch, enabled = true) {
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
