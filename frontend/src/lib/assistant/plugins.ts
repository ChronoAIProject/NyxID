// Derivation logic for the assistant Plugins marketplace (Connectors tab).
// Merges the user's real connected services (`GET /api/v1/keys`) with the
// service catalog (`GET /api/v1/catalog`): connected rows render as "Added",
// catalog entries with no matching connection as "Available to add".

import type { CatalogEntry, KeyInfo } from "@/types/keys";
import type { ConnectCardContentBlock } from "@/types/assistant";

export type PluginKind = "connector" | "skill";

export interface ConnectorCardItem {
  readonly id: string;
  readonly name: string;
  readonly initial: string;
  /** Catalog slug for the shared ServiceIcon brand glyph; null = custom
   *  service with no catalog backing (tile falls back to the initial). */
  readonly iconSlug: string | null;
  readonly category: string;
  readonly description: string;
  /** Footer meta shown for not-yet-added items (e.g. "api key", "oauth"). */
  readonly meta: string;
  readonly added: boolean;
  /**
   * Added items only: every UserService id behind this card. A service with
   * several credentials keeps them all here so the manage modal can show them
   * together rather than handing the user off to the Studio keys list.
   */
  readonly manageKeyIds?: readonly string[];
  /** Available items only: catalog slug for the `/keys?slug=` connect deep link. */
  readonly connectSlug?: string;
}

export interface ConnectorItems {
  readonly added: readonly ConnectorCardItem[];
  readonly available: readonly ConnectorCardItem[];
}

/** Tile initial: first letters of the first two words ("OpenClaw Gateway"
 *  → "OG"), or the first letter for single-word names ("Stripe" → "S"). */
export function connectorInitial(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const first = words[0]?.charAt(0) ?? "?";
  const second = words.length > 1 ? (words[1]?.charAt(0) ?? "") : "";
  return `${first}${second}`.toUpperCase();
}

/** Lowercase auth-kind meta for a catalog entry, mirroring the connect flow
 *  the entry drives (same shape detection as the CLI wizard catalog grid). */
function catalogMeta(entry: CatalogEntry): string {
  if (entry.service_type === "ssh") return "ssh";
  const providerType = (entry.provider_type ?? "").toLowerCase();
  if (providerType === "oauth2") return "oauth";
  if (providerType === "device_code") return "device code";
  if (!entry.requires_credential) return "no credential";
  if (
    entry.token_exchange_credential_fields &&
    entry.token_exchange_credential_fields.length > 0
  ) {
    return "token exchange";
  }
  if (entry.requires_gateway_url) return "self-hosted";
  return "api key";
}

function categoryOf(serviceType: string): string {
  return serviceType === "ssh" ? "Connector · SSH" : "Connector";
}

/**
 * Which connect modality a catalog entry actually drives.
 *
 * The assistant's `connect_card` block carries an `auth_kind`, but it is
 * display data only — the live authorization frame doesn't know the modality,
 * so the transport fills in `"api_key"` as a placeholder. The card must
 * resolve the real one from the catalog before choosing an affordance, or an
 * OAuth service gets offered a paste-your-key button.
 *
 * Same detection as `catalogMeta` above, narrowed to the three modalities the
 * card renders. Anything that isn't OAuth or device code is handled by the
 * credential form, so it maps to `api_key`.
 */
export function catalogAuthKind(
  entry: CatalogEntry,
): ConnectCardContentBlock["auth_kind"] {
  const providerType = (entry.provider_type ?? "").toLowerCase();
  if (providerType === "oauth2") return "oauth";
  if (providerType === "device_code") return "device_code";
  return "api_key";
}

/** One card per connected SERVICE: a service with several credentials still
 *  renders once (no duplicates), with the connection count as its meta. */
function addedServiceItem(
  slug: string,
  group: readonly KeyInfo[],
  catalogBySlug: ReadonlyMap<string, CatalogEntry>,
): ConnectorCardItem {
  const first = group[0] as KeyInfo;
  const catalogEntry = catalogBySlug.get(slug);
  const name = catalogEntry?.name ?? first.label;
  return {
    id: `service:${slug}`,
    name,
    initial: connectorInitial(name),
    iconSlug: slug,
    category: categoryOf(first.service_type),
    description:
      catalogEntry?.description ??
      first.description ??
      `Connected through the NyxID proxy (${first.endpoint_url}).`,
    meta:
      group.length > 1
        ? `${String(group.length)} connections`
        : first.credential_type.replaceAll("_", " "),
    added: true,
    manageKeyIds: group.map((key) => key.id),
  };
}

function addedCustomItem(key: KeyInfo): ConnectorCardItem {
  return {
    id: `key:${key.id}`,
    name: key.label,
    initial: connectorInitial(key.label),
    iconSlug: null,
    category: categoryOf(key.service_type),
    description:
      key.description ??
      `Connected through the NyxID proxy (${key.endpoint_url}).`,
    meta: key.credential_type.replaceAll("_", " "),
    added: true,
    manageKeyIds: [key.id],
  };
}

function availableItem(entry: CatalogEntry): ConnectorCardItem {
  return {
    id: `catalog:${entry.slug}`,
    name: entry.name,
    initial: connectorInitial(entry.name),
    iconSlug: entry.slug,
    category: categoryOf(entry.service_type),
    description: entry.description ?? `Proxy requests to ${entry.base_url}.`,
    meta: catalogMeta(entry),
    added: false,
    connectSlug: entry.slug,
  };
}

/**
 * Split the marketplace into Added (one card per connected service — several
 * credentials for the same service dedupe into a single card with a
 * connection count; custom services with no catalog slug get one card per
 * key) and Available to add (catalog entries with no connection). The join
 * key is `catalog_service_slug` — never `KeyInfo.slug`, which is the
 * user-facing proxy path and can diverge from the catalog entry (custom
 * labels, auto-suffixing).
 */
export function deriveConnectorItems(
  keys: readonly KeyInfo[],
  catalog: readonly CatalogEntry[],
): ConnectorItems {
  const catalogBySlug = new Map(catalog.map((entry) => [entry.slug, entry]));
  const bySlug = new Map<string, KeyInfo[]>();
  const custom: KeyInfo[] = [];
  for (const key of keys) {
    if (key.catalog_service_slug) {
      const group = bySlug.get(key.catalog_service_slug) ?? [];
      group.push(key);
      bySlug.set(key.catalog_service_slug, group);
    } else {
      custom.push(key);
    }
  }
  return {
    added: [
      ...[...bySlug.entries()].map(([slug, group]) =>
        addedServiceItem(slug, group, catalogBySlug),
      ),
      ...custom.map(addedCustomItem),
    ],
    available: catalog
      .filter((entry) => !bySlug.has(entry.slug))
      .map(availableItem),
  };
}

/** Case-insensitive marketplace search over the card's visible text. */
export function matchesPluginQuery(
  item: Pick<ConnectorCardItem, "name" | "category" | "description">,
  query: string,
): boolean {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return true;
  return `${item.name} ${item.category} ${item.description}`
    .toLowerCase()
    .includes(normalized);
}
