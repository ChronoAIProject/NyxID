import { api } from "@/lib/api-client";
import { credentialLabel } from "@/lib/service-insights";
import {
  configuredAgentKeyListSchema,
  configuredBindingsSchema,
  configuredOrgListSchema,
  configuredCatalogSchema,
  type ConfiguredAgentKey,
  type ConfiguredCatalogEntry,
  type ServiceBillingExplanation,
  type ServiceInsight,
} from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";

// Read only metadata from the deployed inventory APIs. This projection describes
// configuration; the execution resolver remains authoritative for ACLs and costs.
type ManagedKey = ConfiguredAgentKey & {
  ownerId: string;
  overrides: ReadonlySet<string> | null;
};

export function configuredBilling(
  connection: KeyInfo,
  catalog?: ConfiguredCatalogEntry,
): ServiceBillingExplanation {
  const platform = connection.credential_binding === "platform";
  const userCredential =
    !platform &&
    Boolean(
      connection.credential_binding === "user" ||
      connection.api_key_id ||
      connection.node_id ||
      connection.has_node_binding,
    );
  const org =
    connection.credential_source?.type === "org"
      ? connection.credential_source
      : null;
  const lane = platform
    ? (connection.platform_key_pricing ?? catalog?.platform_key?.pricing)
    : userCredential
      ? (connection.byok_pricing ?? catalog?.byok_pricing)
      : undefined;
  return {
    status: "conditional",
    credential_class: null,
    credential_label:
      platform || userCredential || connection.auth_method === "none"
        ? credentialLabel(connection)
        : "Connection default · unverified",
    account: null,
    payer_rule: platform
      ? "Acting user's personal account"
      : !userCredential
        ? "Determined at execution"
        : org
          ? `${org.org_name} · organization`
          : "Your personal account",
    charge_status: "conditional",
    rates: lane
      ? [lane, ...(lane.components ?? [])].map((rate) => ({
          layer: "platform",
          metric: rate.metric,
          credits_per_unit: rate.credits_per_unit,
          currency: "credits",
          source: "configuration",
          sync_status: rate.sync_status ?? "unknown",
        }))
      : [],
    provider_billing: platform
      ? "nyxid_credential"
      : connection.auth_method === "none"
        ? "no_credential"
        : userCredential
          ? "separate_provider_account"
          : "unknown",
    context: "configuration",
    notes: [
      "Configured billing for the connection default. The payer and applicable charges are verified at execution; agent credential overrides can change them.",
      ...(lane &&
      [lane, ...(lane.components ?? [])].some(
        (rate) => rate.sync_status !== "synced",
      )
        ? [
            "Unsynced prices are not confirmed as active. Current billing rules apply until price synchronization completes.",
          ]
        : []),
      ...(lane
        ? []
        : [
            "This server does not report a credential-specific rate here. This does not mean usage is free.",
          ]),
    ],
  };
}

function permission(
  key: ManagedKey,
  connection: KeyInfo,
  actorId: string,
): string | null {
  const org =
    connection.credential_source?.type === "org"
      ? connection.credential_source
      : null;
  const ownerId = org?.org_id ?? actorId;
  if (key.ownerId !== ownerId && !(key.ownerId === actorId && org?.allowed))
    return null;
  if (key.allow_all_services) return "all_services";
  if (key.allowed_service_ids.includes(connection.id))
    return "selected_service";
  if (
    key.ownerId === ownerId &&
    key.allow_auto_connected_services &&
    (connection.auto_connected || connection.credential_binding === "platform")
  )
    return "platform_services";
  return null;
}

export async function loadConfiguredServiceInsights(
  connections: readonly KeyInfo[],
  actorId: string,
): Promise<ServiceInsight[]> {
  const [personal, organizations, catalog] = await Promise.allSettled([
    api
      .get<unknown>("/api-keys")
      .then((data) => configuredAgentKeyListSchema.parse(data).keys),
    api
      .get<unknown>("/orgs")
      .then((data) => configuredOrgListSchema.parse(data).orgs),
    api
      .get<unknown>("/catalog")
      .then((data) => configuredCatalogSchema.parse(data).entries),
  ]);
  const relevantOrgs = new Set(
    connections.flatMap((connection) =>
      connection.credential_source?.type === "org"
        ? [connection.credential_source.org_id]
        : [],
    ),
  );
  const adminIds =
    organizations.status === "fulfilled"
      ? organizations.value
          .filter(
            (org) => org.your_role === "admin" && relevantOrgs.has(org.id),
          )
          .map((org) => org.id)
      : [];
  const orgKeys = await Promise.allSettled(
    adminIds.map(async (id) => ({
      ownerId: id,
      keys: configuredAgentKeyListSchema.parse(
        await api.get<unknown>(`/api-keys?org_id=${encodeURIComponent(id)}`),
      ).keys,
    })),
  );
  const inventories = [
    ...(personal.status === "fulfilled"
      ? [{ ownerId: actorId, keys: personal.value }]
      : []),
    ...orgKeys.flatMap((result) =>
      result.status === "fulfilled" ? [result.value] : [],
    ),
  ];
  const now = Date.now();
  const managed: ManagedKey[] = inventories.flatMap(({ ownerId, keys }) =>
    keys
      .filter(
        (key) =>
          key.purpose === "general" &&
          key.is_active &&
          (!key.expires_at || Date.parse(key.expires_at) > now) &&
          key.scopes
            .split(/\s+/)
            .some((scope) => ["proxy", "proxy:*", "llm:proxy"].includes(scope)),
      )
      .map((key) => ({
        ...key,
        ownerId,
        overrides: key.bindings_count === 0 ? new Set<string>() : null,
      })),
  );
  const withOverrides = managed.filter(
    (key) =>
      key.bindings_count > 0 &&
      connections.some((connection) => permission(key, connection, actorId)),
  );
  // Bound fanout for accounts with many agent keys; each request is shared
  // across all visible connections rather than repeated per card.
  for (let i = 0; i < withOverrides.length; i += 6) {
    await Promise.allSettled(
      withOverrides.slice(i, i + 6).map(async (key) => {
        const data = configuredBindingsSchema.parse(
          await api.get<unknown>(
            `/api-keys/${encodeURIComponent(key.id)}/bindings`,
          ),
        );
        key.overrides = new Set(
          data.bindings
            .filter((binding) => binding.api_key_id === key.id)
            .map((binding) => binding.user_service_id),
        );
      }),
    );
  }
  const prices = new Map(
    catalog.status === "fulfilled"
      ? catalog.value.map((entry) => [entry.slug, entry])
      : [],
  );
  return connections.map((connection) => {
    const orgId =
      connection.credential_source?.type === "org"
        ? connection.credential_source.org_id
        : null;
    const orgIndex = orgId ? adminIds.indexOf(orgId) : -1;
    const incomplete =
      personal.status !== "fulfilled" ||
      organizations.status !== "fulfilled" ||
      (orgIndex >= 0 && orgKeys[orgIndex]?.status !== "fulfilled") ||
      inventories.some((inventory) =>
        inventory.keys.some((key) => !key.purpose),
      );
    const keys = managed.flatMap((key) => {
      const reason = permission(key, connection, actorId);
      return reason
        ? [
            {
              id: key.id,
              name: key.name,
              platform: key.platform ?? null,
              owner_id: key.ownerId,
              permission: reason,
              credential_override: key.overrides?.has(connection.id) ?? null,
            },
          ]
        : [];
    });
    const price = prices.get(connection.catalog_service_slug ?? "");
    return {
      service_id: connection.id,
      billing: configuredBilling(connection, price),
      usage: {
        access: {
          visibility:
            personal.status !== "fulfilled" && !inventories.length
              ? "unavailable"
              : orgIndex >= 0
                ? "managed_keys"
                : "own_keys",
          basis: "configuration",
          incomplete,
          keys,
          truncated: false,
        },
        activity: {
          visibility: "unavailable",
          tracking: "unavailable",
          period_days: 30,
          request_count: 0,
          requests: [],
          truncated: false,
        },
      },
    };
  });
}
