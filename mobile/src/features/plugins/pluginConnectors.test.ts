import assert from "node:assert/strict";
import test from "node:test";
import type {
  CatalogEntry,
  ServiceConnection,
} from "../services/serviceContracts";
import { deriveConnectorItems, matchesPluginQuery } from "./pluginConnectors";

const connection: ServiceConnection = {
  id: "github-personal",
  label: "My GitHub",
  slug: "api-github-custom-2",
  catalog_service_slug: "api-github",
  credential_type: "oauth2",
  is_active: true,
  auto_connected: false,
  status: "active",
  credential_source: { type: "personal" },
};
const catalog: CatalogEntry = {
  slug: "api-github",
  name: "GitHub",
  description: "Issues and source code.",
  service_type: "http",
  auth_method: "bearer",
  auth_key_name: "Authorization",
  requires_credential: true,
  requires_gateway_url: false,
  provider_type: "oauth2",
};

test("canonical catalog slugs group every credential despite divergent proxy paths", () => {
  const other = { ...connection, id: "github-work", slug: "another-route" };
  const unrelated = {
    ...catalog,
    slug: connection.slug,
    name: "Other service",
  };
  const result = deriveConnectorItems(
    [connection, other],
    [unrelated, catalog],
  );
  assert.equal(result.added.length, 1);
  assert.equal(result.added[0]?.id, "service:api-github");
  assert.equal(result.added[0]?.name, "GitHub");
  assert.deepEqual(result.added[0]?.manageKeyIds, [connection.id, other.id]);
  assert.deepEqual(result.added[0]?.connections, [connection, other]);
  assert.equal(result.added[0]?.meta, "2 connections");
  assert.equal(result.added[0]?.summary, "Personal · OAuth · 2 connections");
  assert.deepEqual(
    result.available.map((item) => item.connectSlug),
    [unrelated.slug],
  );
});

test("production instance suffixes never replace the canonical catalog brand slug", () => {
  const slugs = ["api-github", "api-notion", "api-google-workspace"];
  const connections = slugs.map((slug, index) => ({
    ...connection,
    id: `connection-${index}`,
    slug: `${slug}-${index + 2}`,
    catalog_service_slug: slug,
  }));
  for (const entries of [[], slugs.map((slug) => ({ ...catalog, slug }))]) {
    const result = deriveConnectorItems(connections, entries);
    assert.deepEqual(
      result.added.map((item) => item.iconSlug),
      slugs,
    );
    assert.deepEqual(
      result.added.map((item) => item.connections[0]?.slug),
      connections.map((item) => item.slug),
    );
    assert.deepEqual(
      result.added.map((item) => item.manageKeyIds?.[0]),
      connections.map((item) => item.id),
    );
  }
});

test("disabled stays Added and mixed connections are Enabled only with a healthy route", () => {
  const off = { ...connection, is_active: false };
  const disabled = deriveConnectorItems([off], [catalog]);
  assert.equal(disabled.added[0]?.disabled, true);
  assert.equal(disabled.added[0]?.status, "Disabled");
  assert.equal(disabled.available.length, 0);
  const mixed = deriveConnectorItems(
    [off, { ...connection, id: "healthy" }],
    [catalog],
  );
  assert.equal(mixed.added[0]?.disabled, false);
  assert.equal(mixed.added[0]?.status, "Enabled");
  assert.equal(mixed.added[0]?.statusTone, "success");
  for (const unhealthy of [
    { ...connection, status: "pending_auth" },
    { ...connection, status: "failed" },
    { ...connection, credential_missing: true },
    { ...connection, connection_status: "expired" as const },
  ]) {
    const result = deriveConnectorItems([off, unhealthy], [catalog]);
    assert.notEqual(result.added[0]?.status, "Enabled");
    assert.notEqual(result.added[0]?.statusTone, "success");
  }
});

test("pending and unavailable credentials report their actual state", () => {
  const status = (patch: Partial<ServiceConnection>) =>
    deriveConnectorItems([{ ...connection, ...patch }], [catalog]).added[0]
      ?.status;
  assert.equal(status({ status: "pending_auth" }), "Authorization needed");
  assert.equal(status({ status: "refresh_failed" }), "Needs attention");
  assert.equal(status({ credential_missing: true }), "Missing credential");
  assert.equal(status({ connection_status: "expired" }), "Expired");
  assert.equal(status({ status: "new_server_status" }), "Check connection");
  assert.equal(
    status({
      credential_source: {
        type: "org",
        org_id: "org",
        org_name: "Work",
        role: "viewer",
        allowed: false,
      },
    }),
    "Restricted",
  );
});

test("custom connections stay independent even when their proxy slugs match a catalog", () => {
  const custom = {
    ...connection,
    catalog_service_slug: null,
    slug: catalog.slug,
  };
  const second = { ...custom, id: "custom-2", label: "Separate credential" };
  const result = deriveConnectorItems([custom, second], [catalog]);
  assert.deepEqual(
    result.added.map((item) => item.id),
    [`key:${custom.id}`, `key:${second.id}`],
  );
  assert.equal(result.available[0]?.connectSlug, catalog.slug);
  assert.equal(result.added[0]?.iconSlug, null);
});

test("metadata reflects the owners and credentials of manual connections", () => {
  const noAuth = deriveConnectorItems(
    [{ ...connection, credential_type: "none" }],
    [],
  ).added[0]!;
  assert.equal(noAuth.meta, "No auth");
  assert.equal(noAuth.summary, "Personal · No auth · 1 connection");
  const org: ServiceConnection = {
    ...connection,
    id: "org-key",
    credential_type: "api_key",
    credential_source: {
      type: "org",
      org_id: "org",
      org_name: "Work",
      role: "admin",
      allowed: true,
    },
  };
  const result = deriveConnectorItems([connection, org], []);
  assert.equal(
    result.added[0]?.summary,
    "Personal + Work · OAuth / API key · 2 connections",
  );
  assert.equal(
    result.added[0]?.description,
    "Connected through the NyxID proxy.",
  );
});

test("automatic connections stay out of Added, management and Discover", () => {
  const automatic = { ...connection, id: "automatic", auto_connected: true };
  const customAutomatic = { ...automatic, id: "automatic-custom", catalog_service_slug: null };
  for (const is_active of [true, false]) {
    const result = deriveConnectorItems([{ ...automatic, is_active }, customAutomatic], [catalog]);
    assert.deepEqual(result.added, []);
    assert.deepEqual(result.available, []);
  }
  const mixed = deriveConnectorItems(
    [automatic, { ...connection, is_active: false }],
    [catalog],
  );
  assert.deepEqual(mixed.added[0]?.manageKeyIds, [connection.id]);
  assert.deepEqual(mixed.added[0]?.connections, [{ ...connection, is_active: false }]);
  assert.equal(mixed.added[0]?.meta, "oauth2");
  assert.equal(mixed.added[0]?.summary, "Personal · OAuth · 1 connection");
  assert.equal(mixed.added[0]?.status, "Disabled");
  assert.deepEqual(mixed.available, []);
});

test("missing catalog still preserves Added while catalog-only cards are available", () => {
  assert.equal(
    deriveConnectorItems([connection], []).added[0]?.name,
    connection.label,
  );
  const result = deriveConnectorItems([], [catalog]);
  assert.equal(result.added.length, 0);
  assert.equal(result.available[0]?.meta, "oauth");
  assert.equal(result.available[0]?.status, "Connect");
  assert.deepEqual(result.available[0]?.connections, []);
});

test("a single catalog connection prefers its override then server catalog icon then fetched catalog icon", () => {
  const override = "https://assets.example/connection.png";
  const serverCatalog = "https://assets.example/server-catalog.svg";
  const fetchedCatalog = "https://assets.example/fetched-catalog.svg";
  const icon = (
    connectionPatch: Partial<ServiceConnection>,
    catalogPatch: Partial<CatalogEntry> = { icon_url: fetchedCatalog },
  ) =>
    deriveConnectorItems(
      [{ ...connection, ...connectionPatch }],
      [{ ...catalog, ...catalogPatch }],
    ).added[0]?.iconUrl;

  assert.equal(
    icon({ icon_url: override, catalog_icon_url: serverCatalog }),
    override,
  );
  assert.equal(
    icon({ icon_url: null, catalog_icon_url: serverCatalog }),
    serverCatalog,
  );
  assert.equal(icon({ catalog_icon_url: serverCatalog }), serverCatalog);
  assert.equal(
    icon({ icon_url: "", catalog_icon_url: serverCatalog }),
    serverCatalog,
  );
  assert.equal(
    icon({ icon_url: null, catalog_icon_url: null }),
    fetchedCatalog,
  );
  assert.equal(icon({ icon_url: "", catalog_icon_url: "" }), fetchedCatalog);
  assert.equal(icon({}, { icon_url: null }), null);
  assert.equal(icon({}, {}), null);
  assert.equal(
    deriveConnectorItems(
      [{ ...connection, catalog_icon_url: serverCatalog }],
      [],
    ).added[0]?.iconUrl,
    serverCatalog,
  );
});

test("grouped credentials share their catalog icon rather than an individual connection override", () => {
  const first = {
    ...connection,
    icon_url: "https://assets.example/personal.png",
    catalog_icon_url: "https://assets.example/server-catalog.svg",
  };
  const second = {
    ...connection,
    id: "github-work",
    icon_url: "https://assets.example/work.png",
    catalog_icon_url: "https://assets.example/other-catalog.svg",
  };
  const fetched = {
    ...catalog,
    icon_url: "https://assets.example/catalog.svg",
  };
  assert.equal(
    deriveConnectorItems([first, second], [fetched]).added[0]?.iconUrl,
    first.catalog_icon_url,
  );
  assert.equal(
    deriveConnectorItems(
      [{ ...first, catalog_icon_url: null }, second],
      [fetched],
    ).added[0]?.iconUrl,
    fetched.icon_url,
  );
  assert.equal(
    deriveConnectorItems(
      [{ ...first, catalog_icon_url: null }, second],
      [catalog],
    ).added[0]?.iconUrl,
    null,
  );
});

test("custom connections use only their own icon and keep existing slug fallback", () => {
  const custom = {
    ...connection,
    catalog_service_slug: null,
    slug: catalog.slug,
    icon_url: "https://assets.example/custom.png",
    catalog_icon_url: "https://assets.example/ignored-catalog.svg",
  };
  const fetched = {
    ...catalog,
    icon_url: "https://assets.example/catalog.svg",
  };
  const result = deriveConnectorItems([custom], [fetched]);
  assert.equal(result.added[0]?.iconSlug, null);
  assert.equal(result.added[0]?.iconUrl, custom.icon_url);
  assert.equal(result.available[0]?.iconUrl, fetched.icon_url);
  for (const icon_url of [undefined, null]) {
    assert.equal(
      deriveConnectorItems([{ ...custom, icon_url }], [fetched]).added[0]
        ?.iconUrl,
      null,
    );
  }
});

test("available cards preserve catalog icons and older omitted/null metadata", () => {
  for (const icon_url of [
    "https://assets.example/catalog.svg",
    null,
    undefined,
  ]) {
    const result = deriveConnectorItems([], [{ ...catalog, icon_url }]);
    assert.equal(result.available[0]?.iconUrl, icon_url);
    assert.equal(result.available[0]?.iconSlug, catalog.slug);
  }
});

test("catalog auth metadata follows Web detection and SSH category", () => {
  const meta = (patch: Partial<CatalogEntry>) =>
    deriveConnectorItems([], [{ ...catalog, ...patch }]).available[0]?.meta;
  assert.equal(meta({ service_type: "ssh" }), "ssh");
  assert.equal(meta({ provider_type: "device_code" }), "device code");
  assert.equal(
    meta({ provider_type: null, requires_credential: false }),
    "no credential",
  );
  assert.equal(
    meta({ provider_type: null, requires_gateway_url: true }),
    "self-hosted",
  );
  assert.equal(meta({ provider_type: null }), "api key");
  assert.equal(
    meta({
      provider_type: null,
      token_exchange_credential_fields: [
        { name: "app", label: "App", secret: false },
      ],
    }),
    "token exchange",
  );
});

test("search matches visible name category and description with trimmed case-insensitive input", () => {
  const item = deriveConnectorItems([connection], [catalog]).added[0]!;
  for (const query of [" GitHUB ", "CONNECTOR", "Source CODE", "   "])
    assert.equal(matchesPluginQuery(item, query), true);
  assert.equal(matchesPluginQuery(item, connection.id), false);
  assert.equal(matchesPluginQuery(item, "no match"), false);
});
