import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { analyticsPath, filterError } from "@/lib/usage-analytics";
import {
  analyticsResponseSchema,
  type AnalyticsFilters,
  type AnalyticsPanel,
  type AnalyticsResult,
} from "@/schemas/usage-analytics";
import { useAuthStore } from "@/stores/auth-store";

/** Matches the backend's per-request id cap on `/admin/usage/identities`. */
const IDENTITY_LABEL_BATCH = 100;

export interface FilterOption {
  id: string;
  label: string;
  detail?: string;
}
export type SampleOptions = Record<
  "services" | "actors" | "owners",
  FilterOption[]
>;

export function useAnalyticsOptions(
  kind: keyof SampleOptions,
  search: string,
  enabled: boolean,
  sample?: SampleOptions,
) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: ["analytics-options", userId, kind, search, Boolean(sample)],
    queryFn: async (): Promise<{ options: FilterOption[]; total: number }> => {
      if (sample) {
        const options = sample[kind].filter((option) =>
          `${option.label} ${option.detail ?? ""}`
            .toLowerCase()
            .includes(search.toLowerCase()),
        );
        return { options, total: options.length };
      }
      if (kind === "services") {
        const result = await api.get<{
          services: { id: string; name: string; slug: string }[];
        }>("/services");
        const options = result.services
          .filter((service) =>
            `${service.name} ${service.slug}`
              .toLowerCase()
              .includes(search.toLowerCase()),
          )
          .map((service) => ({
            id: service.id,
            label: service.name,
            detail: service.slug,
          }));
        return { options, total: options.length };
      }
      return api.get<{ options: FilterOption[]; total: number }>(
        `/admin/usage/identities?${new URLSearchParams({ kind, search })}`,
      );
    },
    enabled,
    staleTime: 5 * 60_000,
    gcTime: 10 * 60_000,
    retry: false,
  });
}

export function useUsageAnalytics(
  filters: AnalyticsFilters,
  panel: AnalyticsPanel,
  sample?: (
    filters: AnalyticsFilters,
    panel: AnalyticsPanel,
  ) => AnalyticsResult,
  enabled = true,
) {
  const userId = useAuthStore((state) => state.user?.id);
  const path = analyticsPath(filters, panel);
  return useQuery({
    queryKey: ["usage-analytics", userId, Boolean(sample), path],
    queryFn: async () =>
      sample
        ? sample(filters, panel)
        : analyticsResponseSchema.parse(await api.get<unknown>(path)),
    enabled: enabled && !filterError(filters),
    staleTime: 30_000,
    retry: false,
  });
}

export function useAnalyticsLabels(
  filters: AnalyticsFilters,
  sample?: SampleOptions,
) {
  const userId = useAuthStore((state) => state.user?.id);
  const services = useAnalyticsOptions(
    "services",
    "",
    filters.services.length > 0,
    sample,
  );
  const ids = [...new Set([...filters.actors, ...filters.owners])].sort();
  const identities = useQuery({
    queryKey: ["analytics-identities", userId, ids],
    queryFn: async () => {
      const chunks: string[][] = [];
      for (let i = 0; i < ids.length; i += IDENTITY_LABEL_BATCH) {
        chunks.push(ids.slice(i, i + IDENTITY_LABEL_BATCH));
      }
      const results = await Promise.all(
        chunks.map((chunk) =>
          api.get<{ options: FilterOption[] }>(
            `/admin/usage/identities?${new URLSearchParams({ ids: chunk.join(",") })}`,
          ),
        ),
      );
      return Object.fromEntries(
        results.flatMap((result) =>
          result.options.map((option) => [option.id, option.label]),
        ),
      );
    },
    enabled: !sample && ids.length > 0,
    staleTime: 60_000,
    retry: false,
  });
  const labels: Record<keyof SampleOptions, Record<string, string>> = {
    services: {},
    actors: {},
    owners: {},
  };
  for (const option of services.data?.options ?? []) {
    labels.services[option.id] = option.label;
    if (option.detail) labels.services[option.detail] = option.label;
  }
  for (const kind of ["actors", "owners"] as const) {
    for (const id of filters[kind]) {
      const identity = identities.data?.[id];
      labels[kind][id] =
        sample?.[kind].find((option) => option.id === id)?.label ??
        identity ??
        id;
    }
  }
  return labels;
}
