import type { KeyInfo, CatalogEntry } from "@/types/keys";
import type { UserServiceResponse } from "@/schemas/keys";
import type { CredentialSource } from "@/schemas/orgs";

export type RoutingTier = "personal" | "org" | "platform" | "unknown";

export interface RoutingCandidate {
  key: KeyInfo;
  tier: RoutingTier;
  owner: string;
  state: "unavailable" | "unverified";
  reason: string;
  source?: CredentialSource;
}

export interface RoutingGroup {
  id: string;
  name: string;
  slug: string;
  canonical: boolean;
  candidates: RoutingCandidate[];
}

export function classifyConnection(
  key: KeyInfo,
  services: readonly UserServiceResponse[],
  now: number,
): RoutingCandidate {
  const source =
    key.credential_source ??
    services.find((service) => service.id === key.id)?.credential_source;
  const isPlatform =
    key.credential_binding === "platform" ||
    (key.credential_binding === undefined && key.auto_connected);
  const tier: RoutingTier = isPlatform
    ? "platform"
    : (source?.type ?? "unknown");
  const owner = isPlatform
    ? "NyxID platform"
    : source?.type === "org"
      ? source.org_name
      : source
        ? "You"
        : "Unknown owner";
  const result = (
    state: RoutingCandidate["state"],
    reason: string,
  ): RoutingCandidate => ({ key, tier, owner, state, reason, source });
  if (!key.is_active) return result("unavailable", "Disabled");
  if (source?.type === "org" && !source.allowed)
    return result("unavailable", "No access");
  if (isPlatform && key.platform_key_available === false)
    return result("unavailable", "Platform unavailable");
  if (key.credential_missing)
    return result("unavailable", "Credential missing");
  if (["revoked", "failed", "refresh_failed"].includes(key.status))
    return result("unavailable", "Reconnect needed");
  if (
    key.status === "pending_auth" ||
    (key.requires_connection && key.connected === false)
  )
    return result("unavailable", "Finish connecting");
  if (key.node_id && key.node_status !== "online")
    return result("unverified", "Node check needed");
  if (key.connection_status === "expired" || key.status === "expired")
    return result("unavailable", "Reconnect needed");
  if (key.expires_at && Date.parse(key.expires_at) <= now) {
    return key.credential_type === "oauth2"
      ? result("unverified", "Refresh needed")
      : result("unavailable", "Credential expired");
  }
  if (tier === "unknown") return result("unverified", "Owner not reported");
  if (
    !isPlatform &&
    !key.api_key_id &&
    !key.node_id &&
    key.auth_method !== "none"
  )
    return result("unverified", "Credential check needed");
  // GET /keys reports configuration and credential state, not a successful
  // provider check. Explicit platform bindings also need live verification.
  return result("unverified", "Not verified");
}

export function buildRoutingGroups(
  keys: readonly KeyInfo[],
  catalog: readonly CatalogEntry[],
  services: readonly UserServiceResponse[],
  now: number,
): RoutingGroup[] {
  const groups = new Map<string, RoutingGroup>();
  for (const key of keys) {
    const id = key.catalog_service_id
      ? `catalog:${key.catalog_service_id}`
      : `connection:${key.id}`;
    let group = groups.get(id);
    if (!group) {
      const entry = catalog.find(
        (item) => item.slug === key.catalog_service_slug,
      );
      group = {
        id,
        name: entry?.name ?? key.catalog_service_name ?? key.label,
        slug: key.catalog_service_slug ?? key.slug,
        canonical: Boolean(
          key.catalog_service_id &&
          key.catalog_service_slug &&
          key.service_type === "http",
        ),
        candidates: [],
      };
      groups.set(id, group);
    }
    group.candidates.push(classifyConnection(key, services, now));
  }
  const rank = { personal: 0, org: 1, platform: 2, unknown: 3 };
  for (const group of groups.values()) {
    group.candidates.sort(
      (a, b) =>
        rank[a.tier] - rank[b.tier] || a.key.label.localeCompare(b.key.label),
    );
  }
  // A catalog entry (including shared OAuth app credentials) is not a
  // connection. Only records returned by /keys create groups or options.
  return [...groups.values()].sort((a, b) => a.name.localeCompare(b.name));
}
