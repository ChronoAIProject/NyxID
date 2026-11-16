import { useRouterState } from "@tanstack/react-router";

/** Read the raw address only once route validation and the rendered match agree. */
export function useSectionSearchParam(key: string) {
  return useRouterState({
    select: (state) => ({
      value: new URLSearchParams(state.location.searchStr).get(key) ?? undefined,
      settled: state.status === "idle" && state.resolvedLocation?.href === state.location.href,
    }),
  });
}
