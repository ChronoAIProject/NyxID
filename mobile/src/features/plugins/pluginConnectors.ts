import type {
  CatalogEntry,
  ServiceConnection,
} from "../services/serviceContracts";

export interface ConnectorCardItem {
  readonly id: string;
  readonly name: string;
  readonly initial: string;
  readonly iconSlug: string | null;
  readonly category: string;
  readonly description: string;
  readonly meta: string;
  readonly added: boolean;
  readonly disabled?: boolean;
  readonly manageKeyIds?: readonly string[];
  readonly connectSlug?: string;
  readonly connections: readonly ServiceConnection[];
  readonly catalogEntry?: CatalogEntry;
  readonly status: string;
  readonly statusTone: "success" | "warning" | "error" | "neutral";
  readonly summary: string;
}

export interface ConnectorItems {
  readonly added: readonly ConnectorCardItem[];
  readonly available: readonly ConnectorCardItem[];
}

function connectorInitial(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  return `${words[0]?.charAt(0) ?? "?"}${words.length > 1 ? (words[1]?.charAt(0) ?? "") : ""}`.toUpperCase();
}

function catalogMeta(entry: CatalogEntry): string {
  if (entry.service_type === "ssh") return "ssh";
  const provider = entry.provider_type?.toLowerCase();
  if (provider === "oauth2") return "oauth";
  if (provider === "device_code") return "device code";
  if (!entry.requires_credential) return "no credential";
  if (entry.token_exchange_credential_fields?.length) return "token exchange";
  if (entry.requires_gateway_url) return "self-hosted";
  return "api key";
}

function healthy(connection: ServiceConnection): boolean {
  return (
    connection.is_active &&
    connection.status === "active" &&
    connection.connection_status !== "expired" &&
    !connection.credential_missing &&
    (connection.credential_source?.type !== "org" ||
      connection.credential_source.allowed)
  );
}

function connectionState(
  connections: readonly ServiceConnection[],
): Pick<ConnectorCardItem, "status" | "statusTone"> {
  if (connections.every((connection) => !connection.is_active))
    return { status: "Disabled", statusTone: "neutral" };
  if (connections.some(healthy))
    return { status: "Enabled", statusTone: "success" };
  const enabled = connections.filter((connection) => connection.is_active);
  if (enabled.some((connection) => connection.credential_missing))
    return { status: "Missing credential", statusTone: "warning" };
  if (
    enabled.some(
      (connection) =>
        connection.credential_source?.type === "org" &&
        !connection.credential_source.allowed,
    )
  )
    return { status: "Restricted", statusTone: "neutral" };
  if (
    enabled.some((connection) =>
      ["failed", "refresh_failed", "revoked", "error"].includes(
        connection.status,
      ),
    )
  )
    return { status: "Needs attention", statusTone: "error" };
  if (
    enabled.some(
      (connection) =>
        connection.status === "expired" ||
        connection.connection_status === "expired",
    )
  )
    return { status: "Expired", statusTone: "warning" };
  if (enabled.some((connection) => connection.status === "pending_auth"))
    return { status: "Authorization needed", statusTone: "warning" };
  return { status: "Check connection", statusTone: "neutral" };
}

function authLabel(type: string): string {
  if (type === "none") return "No auth";
  if (type === "oauth2") return "OAuth";
  if (type === "api_key") return "API key";
  return type.replaceAll("_", " ");
}

function addedItem(
  id: string,
  connections: readonly ServiceConnection[],
  catalogEntry: CatalogEntry | undefined,
  iconSlug: string | null,
): ConnectorCardItem {
  const first = connections[0]!;
  const name = catalogEntry?.name ?? first.label;
  const owners = [
    ...new Set(
      connections.map((connection) =>
        connection.credential_source?.type === "org"
          ? connection.credential_source.org_name
          : "Personal",
      ),
    ),
  ].join(" + ");
  const auth = [
    ...new Set(
      connections.map((connection) => authLabel(connection.credential_type)),
    ),
  ].join(" / ");
  const count = `${connections.length} ${connections.length === 1 ? "connection" : "connections"}`;
  const endpointDescription =
    !first.auto_connected && first.endpoint_url
      ? `Connected through the NyxID proxy (${first.endpoint_url}).`
      : "Connected through the NyxID proxy.";
  return {
    id,
    name,
    initial: connectorInitial(name),
    iconSlug,
    category: first.service_type === "ssh" ? "Connector · SSH" : "Connector",
    description:
      catalogEntry?.description ?? first.description ?? endpointDescription,
    meta:
      connections.length > 1
        ? count
        : first.credential_type === "none"
          ? "No auth"
          : first.credential_type.replaceAll("_", " "),
    added: true,
    disabled: connections.every((connection) => !connection.is_active),
    manageKeyIds: connections.map((connection) => connection.id),
    connections,
    catalogEntry,
    ...connectionState(connections),
    summary: `${owners} · ${auth} · ${count}`,
  };
}

/** Mirrors Web Plugins: join the canonical catalog slug, keep every credential,
 * and leave disabled connections in Added so Connect does not create duplicates. */
export function deriveConnectorItems(
  connections: readonly ServiceConnection[],
  catalog: readonly CatalogEntry[],
): ConnectorItems {
  const catalogBySlug = new Map(catalog.map((entry) => [entry.slug, entry]));
  const bySlug = new Map<string, ServiceConnection[]>();
  const custom: ServiceConnection[] = [];
  for (const connection of connections) {
    const slug = connection.catalog_service_slug;
    if (slug) {
      const group = bySlug.get(slug) ?? [];
      group.push(connection);
      bySlug.set(slug, group);
    } else custom.push(connection);
  }
  return {
    added: [
      ...[...bySlug].map(([slug, group]) =>
        addedItem(`service:${slug}`, group, catalogBySlug.get(slug), slug),
      ),
      ...custom.map((connection) =>
        addedItem(`key:${connection.id}`, [connection], undefined, null),
      ),
    ],
    available: catalog
      .filter((entry) => !bySlug.has(entry.slug))
      .map((entry) => ({
        id: `catalog:${entry.slug}`,
        name: entry.name,
        initial: connectorInitial(entry.name),
        iconSlug: entry.slug,
        category:
          entry.service_type === "ssh" ? "Connector · SSH" : "Connector",
        description:
          entry.description ??
          (entry.base_url
            ? `Proxy requests to ${entry.base_url}.`
            : "Connect this service through the NyxID proxy."),
        meta: catalogMeta(entry),
        added: false,
        connectSlug: entry.slug,
        connections: [],
        catalogEntry: entry,
        status: "Connect",
        statusTone: "neutral" as const,
        summary: catalogMeta(entry),
      })),
  };
}

/** Case-insensitive search over the same visible card fields Web searches. */
export function matchesPluginQuery(
  item: Pick<ConnectorCardItem, "name" | "category" | "description">,
  query: string,
): boolean {
  const normalized = query.trim().toLowerCase();
  return (
    !normalized ||
    `${item.name} ${item.category} ${item.description}`
      .toLowerCase()
      .includes(normalized)
  );
}
