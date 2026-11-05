import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ApiError } from "@/lib/api-client";

export function crmAuthorityRefused(error: unknown): boolean {
  return error instanceof ApiError && [401, 403, 404].includes(error.status);
}

/** A successful stale cache entry must never outlive an authorization refusal. */
export function useVisibleCrmQuery<
  T extends { data: unknown; isError: boolean; error: unknown },
>(result: T, owner: string | undefined): T {
  const client = useQueryClient();
  useEffect(() => {
    if (!crmAuthorityRefused(result.error)) return;
    if (result.error instanceof ApiError && result.error.status === 404) {
      for (const query of client
        .getQueryCache()
        .findAll({ queryKey: ["crm", owner] })) {
        if (query.state.error !== result.error) continue;
        const prefix =
          query.queryKey[2] === "record"
            ? query.queryKey.slice(0, 4)
            : query.queryKey;
        client.removeQueries({
          queryKey: prefix,
          exact: query.queryKey[2] !== "record",
        });
        if (query.queryKey[2] === "record") {
          const recordId = query.queryKey[3];
          client.removeQueries({
            queryKey: ["crm", owner],
            predicate: (entry) =>
              entry.queryKey[2] === "table" ||
              entry.queryKey[2] === "aggregate" ||
              (entry.queryKey[2] === "record" &&
                entry.queryKey[4] === "related"),
          });
          client.setQueriesData(
            { queryKey: ["crm", owner, "record"] },
            (data: unknown) => {
              if (
                !data ||
                typeof data !== "object" ||
                !("links" in data) ||
                !data.links ||
                typeof data.links !== "object"
              )
                return data;
              return {
                ...data,
                links: Object.fromEntries(
                  Object.entries(data.links).map(
                    ([key, links]: [string, unknown]) => [
                      key,
                      Array.isArray(links)
                        ? links.filter(
                            (link: unknown) =>
                              !link ||
                              typeof link !== "object" ||
                              !("id" in link) ||
                              link.id !== recordId,
                          )
                        : links,
                    ],
                  ),
                ),
              };
            },
          );
        }
      }
    } else client.removeQueries({ queryKey: ["crm", owner] });
  }, [client, owner, result.error]);
  return crmAuthorityRefused(result.error)
    ? { ...result, data: undefined }
    : result;
}
