import { z } from "zod";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";
import { API_KEY_SCOPES } from "@/schemas/api-keys";
import {
  effectiveLoginServiceSchema,
  loginKeyExpiry,
} from "@/schemas/agent-key-login";
import type {
  AgentKeyOptions,
  AgentKeySummary,
} from "@/schemas/agent-key-login";
import type { RequestedPermissions } from "@/schemas/login-request";

export const loginConnectionsSchema = z.object({
  keys: z.array(effectiveLoginServiceSchema),
});
export const loginCatalogSchema = z.object({
  entries: z.array(
    z.object({
      slug: z.string(),
      name: z.string(),
      scope_catalog: z
        .array(
          z.object({
            scope: z.string(),
            label: z.string(),
            description: z.string(),
          }),
        )
        .nullable()
        .optional(),
      default_scopes: z.array(z.string()).nullable().optional(),
    }),
  ),
});
export type LoginConnection = z.infer<
  typeof loginConnectionsSchema
>["keys"][number];
export type LoginCatalog = z.infer<typeof loginCatalogSchema>["entries"];
export type LoginInventory = {
  options: AgentKeyOptions;
  connections: LoginConnection[];
  catalog: LoginCatalog;
};
export type PermissionOption = {
  id: string;
  group: string;
  service: string;
  label: string;
  description: string;
  field: keyof RequestedPermissions;
  value: string;
};
export type AccessEntry = {
  group: string;
  service: string;
  label: string;
  scope?: string;
  description?: string;
};
export type PermissionComparison = {
  matches: boolean;
  exact: boolean;
  matched: AccessEntry[];
  extras: AccessEntry[];
  missing: string[];
};
export const emptyRequested = (): RequestedPermissions => ({
  permissions: [],
  services: [],
  service_permissions: [],
});

export function canonicalScope(slug: string, scope: string): string {
  return slug.startsWith("api-google")
    ? scope.replace(
        /^https:\/\/www\.googleapis\.com\/auth\/userinfo\.(email|profile)$/,
        "$1",
      )
    : scope;
}
export function connectionSlug(connection: LoginConnection) {
  return connection.catalog_service_slug ?? connection.slug;
}
export function connectionReady(connection: LoginConnection, now = Date.now()) {
  return (
    connection.is_active &&
    connection.status === "active" &&
    !connection.credential_missing &&
    connection.connection_status !== "expired" &&
    (!connection.expires_at || Date.parse(connection.expires_at) > now)
  );
}
export function permissionOptions(
  inventory: LoginInventory,
): PermissionOption[] {
  const groups = new Map<
    string,
    {
      name: string;
      scopes: Map<string, { label: string; description: string }>;
    }
  >();
  for (const item of inventory.catalog) {
    const scopes = new Map<string, { label: string; description: string }>();
    for (const scope of item.scope_catalog ?? [])
      scopes.set(canonicalScope(item.slug, scope.scope), scope);
    for (const scope of item.default_scopes ?? [])
      if (!scopes.has(canonicalScope(item.slug, scope)))
        scopes.set(canonicalScope(item.slug, scope), {
          label: scope,
          description: "Published provider permission",
        });
    groups.set(item.slug, { name: item.name, scopes });
  }
  for (const item of inventory.connections) {
    const slug = connectionSlug(item);
    const group = groups.get(slug) ?? {
      name: item.label,
      scopes: new Map<string, { label: string; description: string }>(),
    };
    for (const scope of item.granted_scopes ?? [])
      if (!group.scopes.has(canonicalScope(slug, scope)))
        group.scopes.set(canonicalScope(slug, scope), {
          label: scope,
          description: "Recorded on an existing connection",
        });
    groups.set(slug, group);
  }
  return [
    ...API_KEY_SCOPES.map((scope) => ({
      id: `nyxid::${scope}`,
      group: "nyxid",
      service: "NyxID",
      label: scope,
      description: "NyxID API permission",
      field: "permissions" as const,
      value: scope,
    })),
    ...[...groups]
      .sort((a, b) => a[1].name.localeCompare(b[1].name))
      .flatMap<PermissionOption>(([slug, group]) =>
        group.scopes.size
          ? [...group.scopes].map(([scope, detail]) => ({
              id: `${slug}::${scope}`,
              group: slug,
              service: group.name,
              label: detail.label,
              description: detail.description,
              field: "service_permissions" as const,
              value: `${slug}::${scope}`,
            }))
          : [
              {
                id: slug,
                group: slug,
                service: group.name,
                label: "Use service",
                description: "Provider permissions are not published",
                field: "services" as const,
                value: slug,
              },
            ],
      ),
  ];
}
export function normalizeRequested(
  requested: RequestedPermissions,
): RequestedPermissions {
  return {
    permissions: [...requested.permissions],
    services: [...requested.services],
    service_permissions: [
      ...new Set(
        requested.service_permissions.map((value) => {
          const [slug = "", scope] = value.split("::");
          return scope === undefined
            ? value
            : `${slug}::${canonicalScope(slug, scope)}`;
        }),
      ),
    ],
  };
}
export function requestedProblems(
  requested: RequestedPermissions,
  inventory: LoginInventory,
): string[] {
  const options = permissionOptions(inventory);
  const normalized = normalizeRequested(requested);
  return [
    ...normalized.permissions.filter(
      (v) => !(API_KEY_SCOPES as readonly string[]).includes(v),
    ),
    ...normalized.services.filter((v) => !options.some((o) => o.group === v)),
    ...normalized.service_permissions.filter(
      (v) =>
        !options.some(
          (o) =>
            o.field === "service_permissions" &&
            (o.value === v ||
              (!v.includes("::") && o.value.endsWith(`::${v}`))),
        ),
    ),
  ];
}
export function requestedGroups(
  requested: RequestedPermissions,
): Map<string, string[]> {
  const groups = new Map(
    requested.services.map((slug) => [slug, [] as string[]]),
  );
  for (const value of normalizeRequested(requested).service_permissions) {
    const [slug = "", scope] = value.split("::");
    if (scope !== undefined)
      groups.set(slug, [...(groups.get(slug) ?? []), scope]);
  }
  return groups;
}
export function connectionCovers(
  connection: LoginConnection,
  scopes: string[],
  requested: RequestedPermissions,
) {
  const slug = connectionSlug(connection);
  const required = [
    ...scopes,
    ...requested.service_permissions.filter((v) => !v.includes("::")),
  ];
  return (
    connectionReady(connection) &&
    required.every((scope) =>
      connection.granted_scopes?.some(
        (v) => canonicalScope(slug, v) === canonicalScope(slug, scope),
      ),
    )
  );
}
export function compareKey(
  key: AgentKeySummary,
  requested: RequestedPermissions,
  inventory: LoginInventory,
): PermissionComparison {
  const missing = requestedProblems(requested, inventory).map(
    (v) => `Unknown permission or service: ${v}`,
  );
  const matched: AccessEntry[] = [],
    extras: AccessEntry[] = [];
  // Read is implicit for API keys; admin implies management write, not provider scopes.
  const scopes = new Set(["read", ...key.scopes.split(/\s+/).filter(Boolean)]);
  if (scopes.has("admin")) scopes.add("write");
  if (scopes.delete("proxy:*")) scopes.add("proxy");
  if (
    (requested.services.length || requested.service_permissions.length) &&
    !scopes.has("proxy")
  )
    missing.push("NyxID proxy permission is required to use services");
  for (const scope of requested.permissions)
    if (!scopes.has(scope)) missing.push(`NyxID · ${scope}`);
  for (const scope of scopes)
    (requested.permissions.includes(scope) ? matched : extras).push({
      group: "nyxid",
      service: "NyxID",
      label: scope,
    });
  const connections = effectiveLoginConnections(key, inventory);
  const allowed = [
    ...new Set([
      ...key.allowed_service_ids,
      ...key.allowed_services.map((s) => s.id),
      ...connections.map((c) => c.id),
    ]),
  ];
  if (!key.created_now && (!key.effective_services || !key.permission_snapshot))
    missing.push("Refresh to load the server-resolved key permissions");
  const groups = requestedGroups(requested);
  for (const [slug, required] of groups) {
    if (
      !connections.some(
        (c) =>
          connectionSlug(c) === slug &&
          connectionCovers(c, required, requested) &&
          (!c.node_id ||
            key.allow_all_nodes ||
            key.allowed_node_ids.includes(c.node_id)),
      )
    )
      missing.push(`Missing usable connection: ${slug}`);
  }
  if (
    requested.service_permissions.some((v) => !v.includes("::")) &&
    !connections.some(
      (c) =>
        connectionCovers(c, [], requested) &&
        (!c.node_id ||
          key.allow_all_nodes ||
          key.allowed_node_ids.includes(c.node_id)),
    )
  )
    missing.push("Missing requested provider permissions");
  const options = permissionOptions(inventory);
  for (const connection of connections) {
    const slug = connectionSlug(connection),
      service = connection.label;
    const required = [
      ...(groups.get(slug) ?? []),
      ...requested.service_permissions.filter((v) => !v.includes("::")),
    ];
    const related = groups.has(slug) || required.length > 0;
    if (!related)
      extras.push({ group: slug, service, label: "Additional service" });
    else if (!required.length)
      matched.push({ group: slug, service, label: "Use service" });
    for (const scope of new Set(connection.granted_scopes ?? [])) {
      (required.includes(canonicalScope(slug, scope)) ? matched : extras).push({
        group: slug,
        service,
        label:
          options.find(
            (o) => o.value === `${slug}::${canonicalScope(slug, scope)}`,
          )?.label ?? scope,
        scope,
        description: options.find(
          (o) => o.value === `${slug}::${canonicalScope(slug, scope)}`,
        )?.description,
      });
    }
    if (connection.granted_scopes == null)
      extras.push({
        group: slug,
        service,
        label: "Provider access not reported",
      });
    if (!connectionReady(connection))
      extras.push({ group: slug, service, label: "Connection unavailable" });
  }
  for (const id of allowed)
    if (!connections.some((c) => c.id === id))
      extras.push({
        group: `missing:${id}`,
        service: key.allowed_services.find((s) => s.id === id)?.name ?? id,
        label: "Service access not reported",
      });
  if (key.allow_all_services)
    extras.push({
      group: "nyxid",
      service: "NyxID",
      label: "All current and future services",
    });
  if (key.allow_auto_connected_services)
    extras.push({
      group: "nyxid",
      service: "NyxID",
      label: "All current and future auto-connected platform services",
    });
  if (key.allow_all_nodes)
    extras.push({
      group: "nyxid",
      service: "NyxID",
      label: "All current and future nodes",
    });
  for (const id of key.allowed_node_ids)
    extras.push({
      group: "nodes",
      service: "Nodes",
      label:
        key.allowed_nodes.find((n) => n.id === id)?.name ??
        `Unreported node ${id}`,
    });
  if (key.expires_at && Date.parse(key.expires_at) <= Date.now())
    missing.push("This Agent Key has expired");
  return {
    matches: !missing.length,
    exact: !missing.length && !extras.length,
    matched,
    extras,
    missing,
  };
}

export function exactConnectionDefaults(
  requested: RequestedPermissions,
  inventory: LoginInventory,
  actor: string,
): string[] {
  const ids: string[] = [];
  for (const [slug, scopes] of requestedGroups(requested)) {
    const candidates = inventory.connections.filter(
      (c) =>
        connectionSlug(c) === slug &&
        !c.node_id &&
        inventory.options.services.some(
          (s) => s.id === c.id && s.owner_id === actor,
        ) &&
        connectionCovers(c, scopes, requested),
    );
    if (
      candidates.length === 1 &&
      candidates[0]!.granted_scopes != null &&
      scopes.length > 0 &&
      candidates[0]!.granted_scopes.every((s) =>
        scopes.includes(canonicalScope(slug, s)),
      )
    )
      ids.push(candidates[0]!.id);
  }
  return ids;
}

export function effectiveLoginConnections(
  key: AgentKeySummary,
  inventory: LoginInventory,
): LoginConnection[] {
  if (!key.created_now) return key.effective_services ?? [];
  return inventory.connections.filter((c) =>
    key.allowed_services.some((s) => s.id === c.id),
  );
}

export function draftSummary(
  data: CreateApiKeyFormData,
  inventory: LoginInventory,
  actor: string,
): AgentKeySummary {
  const owner =
    data.target_org_id ?? inventory.options.personal_owner_id ?? actor;
  return {
    id: "",
    name: data.name,
    key_prefix: "",
    owner_id: owner,
    owner_type: data.target_org_id ? "org" : "personal",
    owner_name:
      inventory.options.orgs.find((o) => o.id === owner)?.name ?? "Personal",
    scopes: data.scopes.join(" "),
    allow_all_services: data.allow_all_services ?? false,
    allow_auto_connected_services: data.allow_auto_connected_services ?? false,
    allow_all_nodes: data.allow_all_nodes ?? false,
    allowed_service_ids: data.allowed_service_ids ?? [],
    allowed_node_ids: data.allowed_node_ids ?? [],
    allowed_services: inventory.options.services.filter(
      (s) =>
        (data.allow_all_services === true &&
          (!data.target_org_id || s.owner_id === owner)) ||
        data.allowed_service_ids?.includes(s.id) ||
        (data.allow_auto_connected_services === true &&
          s.auto_connected === true &&
          s.owner_id === owner),
    ),
    allowed_nodes: inventory.options.nodes.filter((s) =>
      data.allowed_node_ids?.includes(s.id),
    ),
    expires_at: loginKeyExpiry(data.expires_at),
    rate_limit_per_second: data.rate_limit_per_second ?? null,
    rate_limit_burst: data.rate_limit_burst ?? null,
    platform: data.platform ?? null,
    created_now: true,
  };
}
