import { useEffect } from "react";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, apiClient } from "@/lib/api-client";
import { optionsResponseSchema } from "@/schemas/options";
import { useAuthStore } from "@/stores/auth-store";
import type { OptionsContext, OptionSet } from "@/types/options";

class OptionsChangedError extends Error {}

export function useOptions(optionSet: OptionSet, context: OptionsContext, search = "") {
  const identity = useAuthStore((state) => state.user?.id);
  const client = useQueryClient();
  const queryKey = ["options", identity, optionSet, context, search] as const;
  const query = useInfiniteQuery({
    queryKey,
    initialPageParam: { offset: 0, version: null as string | null, pageCount: 1 },
    queryFn: async ({ pageParam, signal }) => {
      if (pageParam.pageCount > 102) throw new Error("Options exceed the supported page limit.");
      const params = new URLSearchParams({
        principal_type: context.principal_type,
        owner_id: context.owner_id,
        search,
        offset: String(pageParam.offset),
        limit: "100",
      });
      if (context.service_account_id) params.set("service_account_id", context.service_account_id);
      const response = optionsResponseSchema.parse(await apiClient<unknown>(
        `/options/${optionSet}?${params.toString()}`, { signal },
      ));
      if (response.owner_id !== context.owner_id || response.service_account_id !== (context.service_account_id ?? null)) {
        throw new Error("Options returned for a different account. Reload and retry.");
      }
      if (pageParam.version !== null && response.version !== pageParam.version) {
        throw new OptionsChangedError("Options changed. Reloading the current choices…");
      }
      if (response.next_offset !== null && (response.next_offset <= pageParam.offset || response.next_offset > 10005)) {
        throw new Error("Options returned invalid pagination. Reload and retry.");
      }
      return response;
    },
    getNextPageParam: (page, pages) => page.next_offset === null ? undefined : { offset: page.next_offset, version: page.version, pageCount: pages.length + 1 },
    enabled: Boolean(identity && context.owner_id),
    staleTime: 0,
    gcTime: 24 * 60 * 60 * 1000,
    refetchOnMount: "always",
    refetchOnWindowFocus: "always",
    refetchInterval: 24 * 60 * 60 * 1000,
    retry: false,
  });
  useEffect(() => client.getMutationCache().subscribe((event) => {
    if (event.type === "updated" && event.action.type === "success") {
      void client.invalidateQueries({ queryKey: ["options", identity] });
    }
  }), [client, identity]);
  useEffect(() => {
    if (query.error instanceof OptionsChangedError) {
      void client.resetQueries({ queryKey: ["options", identity, optionSet, context, search], exact: true });
    }
  }, [client, query.error, identity, optionSet, context, search]);
  return {
    ...query,
    versionChanged: query.error instanceof OptionsChangedError,
    retainPartialData: query.isFetchNextPageError && (query.error instanceof TypeError || (query.error instanceof ApiError && query.error.status >= 500)),
  };
}
