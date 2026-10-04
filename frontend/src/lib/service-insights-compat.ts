import { api } from "@/lib/api-client";
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
import {
  configuredUsageCharge,
  configuredPlatformPrice,
  connectionCredentialClass,
  credentialSupplier,
  serviceBillingConfigured,
} from "./service-billing-config";

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
  const credentialClass = connectionCredentialClass(connection);
  const platform = credentialClass === "nyxid_managed_master";
  const supplier = credentialSupplier(connection);
  const sharedOAuth = credentialClass === "nyxid_platform_oauth_app";
  const serviceConfigured = serviceBillingConfigured(connection, catalog);
  const node = Boolean(connection.node_id || connection.has_node_binding);
  const userCredential =
    !platform &&
    (connection.auth_method !== "none" || node) &&
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
  const oauth = ["oauth2", "device_code"].includes(connection.credential_type);
  const billing = catalog?.billing;
  const configuredLane = platform
    ? configuredPlatformPrice(connection, catalog)
    : userCredential
      ? (connection.byok_pricing ??
        catalog?.byok_pricing ??
        billing?.byok_pricing)
      : undefined;
  const hasLanes = Boolean(
    configuredLane ||
    connection.byok_pricing ||
    connection.platform_key_pricing ||
    catalog?.byok_pricing ||
    catalog?.platform_key?.pricing ||
    billing?.byok_pricing ||
    billing?.platform_key_pricing,
  );
  const ownApiKey =
    credentialClass === "user_owned" &&
    connection.credential_type === "api_key";
  const ownOAuthApp = oauth && supplier === "own";
  const excludedFromPlatformCharge =
    billing?.platform_charge_nyxid_credentials_only === true &&
    !platform &&
    supplier !== "nyxid" &&
    (supplier === "own" || supplier === "none");
  const lane = excludedFromPlatformCharge ? undefined : configuredLane;
  const legacyConfigured =
    !hasLanes &&
    !excludedFromPlatformCharge &&
    billing?.platform_billable === true;
  const credentialLabel = platform
    ? "NyxID key"
    : sharedOAuth
      ? "NyxID OAuth app"
      : node
        ? supplier === "own"
          ? "Node credential"
          : "Node credential · supplier unverified"
        : connection.auth_method === "none"
          ? "No credential"
          : ownOAuthApp
            ? `${org ? "Organization" : "Your"} OAuth app (BYOK)`
            : oauth
              ? "Connected account · app unverified"
              : ownApiKey
                ? `${org ? "Organization" : "Your"} API key (BYOK)`
                : supplier === "own"
                  ? `${org ? "Organization" : "Your"} credential (BYOK)`
                  : "Credential supplier unverified";
  const creditBillingConfigured = configuredUsageCharge(connection, catalog);
  return {
    status: "conditional",
    credential_class: credentialClass ?? null,
    credential_label: credentialLabel,
    account: null,
    payer_rule:
      creditBillingConfigured === false
        ? "Not billable by NyxID"
        : platform
          ? "Acting user's personal account"
          : !userCredential
            ? "Determined at execution"
            : org
              ? `${org.org_name} · organization`
              : "Your personal account",
    charge_status:
      creditBillingConfigured === false ? "not_charged" : "conditional",
    credit_billing_configured: creditBillingConfigured,
    service_billing_configured: serviceConfigured,
    credential_supplier: supplier,
    rates:
      lane &&
      creditBillingConfigured !== undefined &&
      (credentialClass || oauth)
        ? [lane, ...(lane.components ?? [])].map((rate) => ({
            layer: "platform",
            metric: rate.metric,
            credits_per_unit: rate.credits_per_unit,
            currency: "credits",
            source: "configuration",
            sync_status: rate.sync_status ?? "unknown",
          }))
        : legacyConfigured &&
            creditBillingConfigured === true &&
            billing?.platform_metric
          ? [
              {
                layer: "platform",
                metric: billing.platform_metric,
                credits_per_unit:
                  billing.platform_pricing?.credits_per_unit ?? null,
                currency: "credits",
                source: "configuration",
                sync_status: billing.platform_pricing?.sync_status ?? "unknown",
              },
            ]
          : [],
    provider_billing:
      platform || sharedOAuth
        ? "nyxid_credential"
        : connection.auth_method === "none"
          ? "no_credential"
          : supplier === "own"
            ? "separate_provider_account"
            : "unknown",
    context: "configuration",
    notes: [
      ...(serviceConfigured === undefined
        ? ["Billing configuration unavailable for this service."]
        : []),
      ...(sharedOAuth && serviceConfigured && creditBillingConfigured === false
        ? [
            "NyxID supplies the OAuth app, but this connection has no configured NyxID usage charge. The service’s platform-key price does not apply to OAuth.",
          ]
        : []),
      creditBillingConfigured === false
        ? "No NyxID usage charges are configured for this connection. The provider may charge separately."
        : "Configured billing for the connection default. The payer and applicable charges are verified at execution; agent credential overrides can change them.",
      ...(oauth && supplier === "unknown"
        ? [
            "Signing in does not identify the developer app's owner. This server does not report whether this connection uses your app or NyxID's app.",
          ]
        : []),
      ...(billing?.platform_charge_nyxid_credentials_only
        ? [
            "Configured NyxID charges are limited to NyxID-supplied credentials or OAuth apps; eligibility must be verified at execution.",
          ]
        : []),
      ...(legacyConfigured
        ? [
            "The catalog configures NyxID credit billing for this service. Caller eligibility and the active plan rate are verified at execution.",
          ]
        : []),
      ...(platform && billing?.resale_billable
        ? [
            "Provider usage through NyxID is configured for credit billing; its rate is not reported here.",
          ]
        : []),
      ...(lane &&
      [lane, ...(lane.components ?? [])].some(
        (rate) => rate.sync_status !== "synced",
      )
        ? [
            "Unsynced prices are not confirmed as active. Current billing rules apply until price synchronization completes.",
          ]
        : []),
      ...(lane || legacyConfigured || creditBillingConfigured === false
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
      .get<unknown>("/catalog?include_all=true")
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
