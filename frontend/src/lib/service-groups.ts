import type { CatalogEntry, KeyInfo } from "@/types/keys";

export interface ServiceConnectionGroup {
  readonly id: string;
  readonly name: string;
  readonly slug: string | null;
  readonly description: string | null;
  readonly connections: readonly KeyInfo[];
}

export function groupServiceConnections(
  keys: readonly KeyInfo[],
  catalog: readonly CatalogEntry[] = [],
): ServiceConnectionGroup[] {
  const groups = new Map<string, KeyInfo[]>();
  for (const key of keys) {
    const id = key.catalog_service_id
      ? `catalog:${key.catalog_service_id}`
      : `connection:${key.id}`;
    const connections = groups.get(id) ?? [];
    connections.push(key);
    groups.set(id, connections);
  }
  return [...groups]
    .map(([id, connections]) => {
      const first = connections[0]!;
      const slug = first.catalog_service_id ? first.catalog_service_slug : null;
      const entry = catalog.find((item) => item.slug === slug);
      return {
        id,
        name: first.catalog_service_id
          ? (entry?.name ?? first.catalog_service_name ?? first.label)
          : first.label,
        slug,
        description:
          entry?.description ??
          (connections.length === 1 ? first.description : null) ??
          null,
        connections,
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name));
}
