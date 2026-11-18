import { createContext } from "react";

// Public queries are opaque visitor context, not JSON route state. Keeping the
// raw string preserves duplicate keys, ordering, and string-looking JSON values.
export const PublicSearchContext = createContext(false);
export function parsePublicSearch(search: string): Record<string, string> {
  return search ? { publicQuery: search } : {};
}
export function stringifyPublicSearch(search: Record<string, unknown>): string {
  return typeof search.publicQuery === "string" ? search.publicQuery : "";
}
