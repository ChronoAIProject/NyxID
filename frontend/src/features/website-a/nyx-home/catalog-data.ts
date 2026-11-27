import catalog from "./production-catalog.json";

export const catalogGroups = catalog.groups;
export const catalogEntries = catalogGroups.flatMap((group) => group.entries);
export const catalogCheckedAt = catalog.checkedAt;
export type CatalogIntegration = (typeof catalogEntries)[number];

export function matchesIntegration(entry: CatalogIntegration, search: string) {
  return [entry.label, entry.detail ?? "", ...entry.catalogSlugs]
    .join(" ")
    .toLowerCase()
    .includes(search.trim().toLowerCase());
}
