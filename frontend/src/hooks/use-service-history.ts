import { useEffect } from "react";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  historyResponseSchema,
  archiveResponseSchema,
} from "@/schemas/service-history";

export function useServiceHistory(id: string, actions: readonly string[]) {
  const identity = useAuthStore((state) => state.user?.id);
  useHistoryInvalidation(identity);
  const query = useInfiniteQuery({
    queryKey: ["service-history", identity, id, actions],
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) => {
      const params = new URLSearchParams({ limit: "20" });
      if (pageParam) params.set("cursor", pageParam);
      if (actions.length) params.set("actions", actions.join(","));
      const response = historyResponseSchema.parse(
        await apiClient<unknown>(
          `/keys/${encodeURIComponent(id)}/history?${params}`,
          { signal },
        ),
      );
      if (response.service_id !== id)
        throw new Error("History returned for a different service.");
      if (response.next_cursor !== null && response.next_cursor === pageParam)
        throw new Error("History returned an invalid cursor.");
      return response;
    },
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: Boolean(identity && id),
    staleTime: 0,
    gcTime: 0,
    retry: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: "always",
  });
  return query;
}

export function useArchivedServiceHistory() {
  const identity = useAuthStore((state) => state.user?.id);
  useHistoryInvalidation(identity);
  return useInfiniteQuery({
    queryKey: ["service-history", identity, "archived"],
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) => {
      const params = new URLSearchParams({ limit: "20" });
      if (pageParam) params.set("cursor", pageParam);
      const response = archiveResponseSchema.parse(
        await apiClient<unknown>(`/keys/history/archived?${params}`, {
          signal,
        }),
      );
      if (response.next_cursor !== null && response.next_cursor === pageParam)
        throw new Error("Invalid archive cursor.");
      return response;
    },
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: Boolean(identity),
    staleTime: 0,
    gcTime: 0,
    retry: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: "always",
  });
}

function useHistoryInvalidation(identity: string | undefined) {
  const client = useQueryClient();
  useEffect(
    () =>
      client.getMutationCache().subscribe((event) => {
        if (event.type === "updated" && event.action.type === "success") {
          void client.invalidateQueries({
            queryKey: ["service-history", identity],
          });
          void client.invalidateQueries({ queryKey: ["keys"] });
        }
      }),
    [client, identity],
  );
}
