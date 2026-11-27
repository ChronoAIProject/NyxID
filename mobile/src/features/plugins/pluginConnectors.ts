import { translateApp } from "../../i18n/appTranslation";
import { describeConnection, isSavedCredential } from "../services/connectionPresentation";
import type {
  CatalogEntry,
  ServiceConnection,
} from "../services/serviceContracts";

export interface ConnectorCardItem {
  readonly id: string;
  readonly name: string;
  readonly iconSlug: string | null;
  readonly iconUrl?: string | null;
  readonly category: string;
  readonly description: string;
  readonly meta: string;
  readonly added: boolean;
  readonly disabled?: boolean;
  readonly manageKeyIds?: readonly string[];
  readonly connectionCountLabel?: string;
  readonly connectSlug?: string;
  readonly connectOwnSlug?: string;
  readonly advancedSetupSlug?: string;
  readonly connections: readonly ServiceConnection[];
  readonly catalogEntry?: CatalogEntry;
  readonly status: string;
  readonly statusTone: "success" | "warning" | "error" | "neutral";
  readonly summary: string;
}

export interface ConnectorItems {
  readonly added: readonly ConnectorCardItem[];
  readonly available: readonly ConnectorCardItem[];
  readonly models: readonly ConnectorCardItem[];
}

function catalogMeta(entry: CatalogEntry): string {
  if (entry.service_type === "ssh") return "ssh";
  const provider = entry.provider_type?.toLowerCase();
  if (provider === "oauth2") return "oauth";
  if (provider === "device_code") return translateApp("device code");
  if (!entry.requires_credential) return translateApp("no credential");
  if (entry.token_exchange_credential_fields?.length) return translateApp("token exchange");
  if (entry.requires_gateway_url) return "self-hosted";
  return translateApp("api key");
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
    return { status: translateApp("Disabled"), statusTone: "neutral" };
  const enabled = connections.filter((connection) => connection.is_active);
  const saved = enabled
    .filter((connection) =>
      isSavedCredential(connection) &&
      (connection.credential_source?.type !== "org" || connection.credential_source.allowed),
    )
    .map(describeConnection);
  if (saved.length) {
    const result = saved.find((item) => item.tone === "error")
      ?? saved.find((item) => item.tone !== "success")
      ?? saved[0]!;
    return {
      status: result.label,
      statusTone: result.tone === "muted" ? "neutral" : result.tone,
    };
  }
  if (connections.some(healthy))
    return { status: translateApp("Connected"), statusTone: "success" };
  if (enabled.some((connection) => connection.credential_missing))
    return { status: translateApp("Missing credential"), statusTone: "warning" };
  if (
    enabled.some(
      (connection) =>
        connection.credential_source?.type === "org" &&
        !connection.credential_source.allowed,
    )
  )
    return { status: translateApp("Restricted"), statusTone: "neutral" };
  if (
    enabled.some((connection) =>
      ["failed", "refresh_failed", "revoked", "error"].includes(
        connection.status,
      ),
    )
  )
    return { status: translateApp("Needs attention"), statusTone: "error" };
  if (
    enabled.some(
      (connection) =>
        connection.status === "expired" ||
        connection.connection_status === "expired",
    )
  )
    return { status: translateApp("Expired"), statusTone: "warning" };
  if (enabled.some((connection) => connection.status === "pending_auth"))
    return { status: translateApp("Authorization needed"), statusTone: "warning" };
  return { status: translateApp("Check connection"), statusTone: "neutral" };
}

function authLabel(type: string): string {
  if (type === "none") return translateApp("No auth");
  if (type === "oauth2") return "OAuth";
  if (type === "api_key") return translateApp("API key");
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
          : translateApp("Personal"),
      ),
    ),
  ].join(" + ");
  const auth = [
    ...new Set(
      connections.map((connection) => authLabel(connection.credential_type)),
    ),
  ].join(" / ");
  const count = translateApp(
    connections.length === 1 ? "{{count}} connection" : "{{count}} connections",
    { count: connections.length },
  );
  const endpointDescription = isSavedCredential(first) ? describeConnection(first).description :
    first.endpoint_url
      ? translateApp("Connected through the NyxID proxy ({{endpointUrl}}).", {
          endpointUrl: first.endpoint_url,
        })
      : translateApp("Connected through the NyxID proxy.");
  return {
    id,
    name,
    iconSlug,
    iconUrl:
      iconSlug === null
        ? (first.icon_url ?? null)
        : (connections.length === 1 ? first.icon_url : null) ||
          first.catalog_icon_url ||
          catalogEntry?.icon_url ||
          null,
    category: first.service_type === "ssh" ? translateApp("Connector · SSH")
        : translateApp("Connector"),
    description:
      catalogEntry?.description?.trim() || first.description?.trim() || endpointDescription,
    meta:
      connections.length > 1
        ? count
        : first.credential_type === "none"
          ? translateApp("No auth")
          : first.credential_type.replaceAll("_", " "),
    added: true,
    disabled: connections.every((connection) => !connection.is_active),
    manageKeyIds: connections.map((connection) => connection.id),
    connectionCountLabel: count,
    connectOwnSlug:
      catalogEntry?.requires_credential && catalogEntry.service_type !== "ssh" &&
      connections.every((connection) => connection.credential_source?.type === "org")
        ? catalogEntry.slug
        : undefined,
    connections,
    catalogEntry,
    ...connectionState(connections),
    summary: translateApp("{{owners}} · {{authentication}} · {{count}}", {
      owners: owners,
      authentication: auth,
      count: count,
    }),
  };
}

function catalogItem(entry: CatalogEntry): ConnectorCardItem {
  const ssh = entry.service_type === "ssh";
  const meta = catalogMeta(entry);
  return {
    id: `catalog:${entry.slug}`,
    name: entry.name,
    iconSlug: entry.slug,
    iconUrl: entry.icon_url,
    category: ssh ? translateApp("Connector · SSH") : translateApp("Connector"),
    description: entry.description?.trim() || (entry.base_url
      ? translateApp("Proxy requests to {{baseUrl}}.", { baseUrl: entry.base_url })
      : translateApp("Connect this service through the NyxID proxy.")),
    meta,
    added: false,
    connectSlug: ssh ? undefined : entry.slug,
    advancedSetupSlug: ssh ? entry.slug : undefined,
    connections: [],
    catalogEntry: entry,
    status: translateApp("Connect"),
    statusTone: "neutral",
    summary: meta,
  };
}

/** Group manual user credentials by catalog slug and keep managed bindings out of Browse. */
export function deriveConnectorItems(
  connections: readonly ServiceConnection[],
  catalog: readonly CatalogEntry[],
): ConnectorItems {
  const catalogBySlug = new Map(catalog.map((entry) => [entry.slug, entry]));
  const connectedSlugs = new Set(
    connections.map((connection) => connection.catalog_service_slug).filter(Boolean),
  );
  const bySlug = new Map<string, ServiceConnection[]>();
  const custom: ServiceConnection[] = [];
  for (const connection of connections) {
    if (connection.auto_connected || connection.credential_binding === "platform") continue;
    const slug = connection.catalog_service_slug;
    if (slug) {
      const group = bySlug.get(slug) ?? [];
      group.push(connection);
      bySlug.set(slug, group);
    } else custom.push(connection);
  }
  const available: ConnectorCardItem[] = [];
  const models: ConnectorCardItem[] = [];
  for (const entry of catalog) {
    if (entry.inference || entry.slug === "chrono-llm") {
      models.push(catalogItem(entry));
    } else if (!connectedSlugs.has(entry.slug) && !entry.platform_key?.available) {
      available.push(catalogItem(entry));
    }
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
    available,
    models,
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
