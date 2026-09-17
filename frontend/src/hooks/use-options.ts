import { useEffect } from "react";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, apiClient } from "@/lib/api-client";
import { optionsResponseSchema } from "@/schemas/options";
import { useAuthStore } from "@/stores/auth-store";
import type { OptionsContext, OptionSet } from "@/types/options";

class OptionsChangedError extends Error {}

export function useOptions(
  optionSet: OptionSet,
  context: OptionsContext,
  search = "",
) {
  const identity = useAuthStore((state) => state.user?.id);
  const client = useQueryClient();
  const queryKey = ["options", identity, optionSet, context, search] as const;
  const query = useInfiniteQuery({
    queryKey,
    initialPageParam: {
      offset: 0,
      version: null as string | null,
      pageCount: 1,
    },
    queryFn: async ({ pageParam, signal }) => {
      if (pageParam.pageCount > (optionSet === "service-scope" ? 102 : 12))
        throw new Error("Options exceed the supported page limit.");
      const params = new URLSearchParams({
        search,
        offset: String(pageParam.offset),
        limit: "100",
      });
      if (context.kind === "service-scope") {
        if (optionSet !== "service-scope")
          throw new Error("Invalid options context.");
        params.set("principal_type", context.principal_type);
        params.set("owner_id", context.owner_id);
        if (context.service_account_id)
          params.set("service_account_id", context.service_account_id);
      } else if (optionSet === "service-scope")
        throw new Error("Invalid options context.");
      const response = optionsResponseSchema.parse(
        await apiClient<unknown>(`/options/${optionSet}?${params.toString()}`, {
          signal,
        }),
      );
      if (
        response.option_set !== optionSet ||
        (context.kind === "service-scope" &&
          (response.option_set !== "service-scope" ||
            response.principal_type !== context.principal_type ||
            response.owner_id !== context.owner_id ||
            response.service_account_id !==
              (context.service_account_id ?? null)))
      ) {
        throw new Error(
          "Options returned for a different account. Reload and retry.",
        );
      }
      if (
        pageParam.version !== null &&
        response.version !== pageParam.version
      ) {
        throw new OptionsChangedError(
          "Options changed. Reloading the current choices…",
        );
      }
      if (
        response.next_offset !== null &&
        (response.next_offset <= pageParam.offset ||
          response.next_offset >= response.total ||
          response.next_offset > (optionSet === "service-scope" ? 10003 : 1000))
      ) {
        throw new Error(
          "Options returned invalid pagination. Reload and retry.",
        );
      }
      return response;
    },
    getNextPageParam: (page, pages) =>
      page.next_offset === null
        ? undefined
        : {
            offset: page.next_offset,
            version: page.version,
            pageCount: pages.length + 1,
          },
    enabled: Boolean(
      identity && (context.kind === "service-history" || context.owner_id),
    ),
    staleTime: 0,
    gcTime: 24 * 60 * 60 * 1000,
    refetchOnMount: "always",
    refetchOnWindowFocus: "always",
    refetchInterval: 24 * 60 * 60 * 1000,
    retry: false,
  });
  useEffect(
    () =>
      client.getMutationCache().subscribe((event) => {
        if (event.type === "updated" && event.action.type === "success") {
          void client.invalidateQueries({ queryKey: ["options", identity] });
        }
      }),
    [client, identity],
  );
  useEffect(() => {
    if (query.error instanceof OptionsChangedError) {
      void client.resetQueries({
        queryKey: ["options", identity, optionSet, context, search],
        exact: true,
      });
    }
  }, [client, query.error, identity, optionSet, context, search]);
  return {
    ...query,
    versionChanged: query.error instanceof OptionsChangedError,
    retainPartialData:
      query.isFetchNextPageError &&
      (query.error instanceof TypeError ||
        (query.error instanceof ApiError && query.error.status >= 500)),
  };
}
