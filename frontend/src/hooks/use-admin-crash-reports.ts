import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  crashListSchema,
  crashReportSchema,
  crashSummarySchema,
  type CrashFilters,
} from "@/schemas/admin-crash-reports";
function query(values: Record<string, string | undefined>): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(values))
    if (value) params.set(key, value);
  return params.toString();
}
export function useCrashSummary(window: string, platform: string) {
  return useQuery({
    queryKey: ["admin", "crash-summary", window, platform],
    queryFn: async () =>
      crashSummarySchema.parse(
        await api.get<unknown>(
          `/admin/crash-reports/summary?${query({ window, platform })}`,
        ),
      ),
    refetchInterval: 60000,
  });
}
export function useCrashReports(filters: CrashFilters) {
  return useQuery({
    queryKey: ["admin", "crash-reports", filters],
    queryFn: async () =>
      crashListSchema.parse(
        await api.get<unknown>(`/admin/crash-reports?${query(filters)}`),
      ),
  });
}
export function useCrashReport(id: string) {
  return useQuery({
    queryKey: ["admin", "crash-report", id],
    queryFn: async () =>
      crashReportSchema.parse(
        await api.get<unknown>(
          `/admin/crash-reports/${encodeURIComponent(id)}`,
        ),
      ),
  });
}

export function useRegisterCrashRelease() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (release: {
      platform: "web" | "ios" | "android";
      app_version: string;
      app_build: string;
      artifact_id?: string;
    }) => api.post("/admin/crash-reports/releases", release),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["admin", "crash-summary"] }),
  });
}
