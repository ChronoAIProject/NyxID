import type { ServiceConnection } from "../services/serviceContracts";
import { connectionOwner } from "../services/serviceContracts";

export const CONNECTION_UNAVAILABLE =
  "The selected connection is unavailable. Choose another connection or remove it before sending.";
/** Exact web connectionAvailable predicate; status/expiry are server-owned. */
export function composerConnectionAvailable(key: ServiceConnection) {
  return (
    key.is_active &&
    !(key.credential_source?.type === "org" && !key.credential_source.allowed)
  );
}
export function composerConnectionGroups(
  keys: readonly ServiceConnection[],
  search: string,
) {
  const groups = new Map<
    string,
    { id: string; name: string; connections: ServiceConnection[] }
  >();
  for (const key of keys) {
    const id = key.catalog_service_id
      ? `catalog:${key.catalog_service_id}`
      : `connection:${key.id}`;
    const group = groups.get(id) ?? {
      id,
      name: key.catalog_service_id
        ? (key.catalog_service_name ?? key.label)
        : key.label,
      connections: [],
    };
    group.connections.push(key);
    groups.set(id, group);
  }
  const query = search.trim().toLocaleLowerCase();
  return [...groups.values()]
    .map((group) => ({
      ...group,
      connections: group.connections.filter((key) =>
        `${group.name} ${key.label} ${key.slug} ${connectionOwner(key)}`
          .toLocaleLowerCase()
          .includes(query),
      ),
    }))
    .filter((group) => group.connections.length)
    .sort((a, b) => a.name.localeCompare(b.name));
}
