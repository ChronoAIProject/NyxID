import type { AnalyticsFilters } from "@/schemas/usage-analytics";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { freeGrantsReportSchema } from "@/schemas/free-grants-report";

export function useFreeGrantsReport(
  params: URLSearchParams,
  page: number,
  servicePage: number,
) {
  const user = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: [
      "admin",
      "free-grants",
      user,
      params.toString(),
      page,
      servicePage,
    ],
    queryFn: async () =>
      freeGrantsReportSchema.parse(
        await api.get(
          `/admin/usage/free-grants?${params}&page=${page}&service_page=${servicePage}`,
        ),
      ),
    placeholderData: (previous, query) =>
      query?.queryKey[2] === user ? previous : undefined,
    retry: false,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false,
  });
}

export function freeGrantParams(filters: AnalyticsFilters) {
  const p = new URLSearchParams();
  if (filters.period === "custom") {
    if (filters.from) p.set("from", filters.from);
    if (filters.to) p.set("to", filters.to);
  } else p.set("period", filters.period);
  if (filters.owners.length)
    p.set("owners", [...filters.owners].sort().join(","));
  return p;
}
