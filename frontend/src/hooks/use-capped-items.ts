import { useState } from "react";
import { useDrawerSearching } from "@/lib/drawer-search";

/** Keep growing lists short; search expands all loaded items and keeps revealed rows mounted. */
export function useCappedItems<T>(items: readonly T[], limit = 5) {
  const [expanded, setExpanded] = useState(false);
  const searching = useDrawerSearching();
  if (searching && !expanded) setExpanded(true);
  return {
    items: expanded || searching ? items : items.slice(0, limit),
    remaining: expanded || searching ? 0 : Math.max(0, items.length - limit),
    showAll: () => setExpanded(true),
  };
}
