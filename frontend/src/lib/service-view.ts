import type { KeyInfo } from "@/types/keys";
import type { ServiceViewFilters } from "@/schemas/service-view";
import type { ServiceConnectionGroup } from "@/lib/service-groups";

export function connectionSource(
  key: KeyInfo,
): "personal" | "org" | "platform" {
  if (
    key.credential_binding === "platform" ||
    (key.credential_binding === undefined && key.auto_connected)
  )
    return "platform";
  return key.credential_source?.type === "org" ? "org" : "personal";
}

export function connectionSourceLabel(key: KeyInfo): string {
  if (connectionSource(key) === "platform") {
    return key.auth_method === "none" ? "Platform managed" : "NyxID platform";
  }
  return key.credential_source?.type === "org"
    ? key.credential_source.org_name
    : "Personal";
}

export function matchingConnections(
  group: ServiceConnectionGroup,
  filters: ServiceViewFilters,
): readonly KeyInfo[] {
  if (
    filters.service_group_ids.length &&
    !filters.service_group_ids.includes(group.id)
  )
    return [];
  const needle = filters.search.trim().toLowerCase();
  const groupMatches = [group.name, group.slug ?? ""].some((value) =>
    value.toLowerCase().includes(needle),
  );
  return group.connections.filter(
    (key) =>
      (!filters.organization_ids.length ||
        (key.credential_source?.type === "org" &&
          filters.organization_ids.includes(key.credential_source.org_id))) &&
      (filters.source === "all" || connectionSource(key) === filters.source) &&
      (filters.state === "all" ||
        (filters.state === "enabled" ? key.is_active : !key.is_active)) &&
      (filters.service_type === "all" ||
        key.service_type === filters.service_type) &&
      (filters.show_auto_connected || !key.auto_connected) &&
      (groupMatches ||
        [key.label, key.slug, connectionSourceLabel(key)].some((value) =>
          value.toLowerCase().includes(needle),
        )),
  );
}
