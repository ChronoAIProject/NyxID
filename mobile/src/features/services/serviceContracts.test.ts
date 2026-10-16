import assert from "node:assert/strict";
import test from "node:test";
import {
  buildCatalogConnectionInput,
  canEditConnection,
  catalogForConnection,
  catalogEntrySchema,
  serviceConnectionSchema,
  supportsManagedOAuth,
  validateBrowserUrl,
  type CatalogEntry,
  type ServiceConnection,
} from "./serviceContracts";

const connection: ServiceConnection = {
  id: "service-1",
  label: "Work GitHub",
  slug: "api-github-2",
  status: "active",
  is_active: true,
  auto_connected: false,
  credential_type: "oauth2",
  catalog_service_slug: "api-github",
  catalog_service_name: "GitHub",
  error_message: null,
  credential_source: { type: "personal" },
};
const entry: CatalogEntry = {
  slug: "api-github",
  name: "GitHub",
  description: null,
  auth_method: "bearer",
  auth_key_name: "Authorization",
  service_type: "http",
  provider_type: "oauth2",
  provider_config_id: "github-provider",
  credential_mode: "both",
  has_platform_oauth_credentials: true,
  requires_credential: true,
  requires_gateway_url: false,
  api_key_instructions: null,
  api_key_url: null,
  documentation_url: null,
};

test("connection editing requires known ownership and preserves read-only platform/org rows", () => {
  assert.equal(canEditConnection(connection), true);
  assert.equal(
    canEditConnection({ ...connection, credential_source: undefined }),
    false,
  );
  assert.equal(
    canEditConnection({
      ...connection,
      credential_source: undefined,
      can_edit_configuration: true,
    }),
    true,
  );
  assert.equal(
    canEditConnection({
      ...connection,
      auto_connected: true,
      can_edit_configuration: true,
    }),
    false,
  );
  assert.equal(
    canEditConnection({ ...connection, auto_connected: true }),
    false,
  );
  assert.equal(
    canEditConnection({ ...connection, can_edit_configuration: false }),
    false,
  );
  const org = {
    type: "org" as const,
    org_id: "org",
    org_name: "Team",
    role: "admin" as const,
    allowed: true,
  };
  assert.equal(
    canEditConnection({ ...connection, credential_source: org }),
    true,
  );
  assert.equal(
    canEditConnection({
      ...connection,
      credential_source: { ...org, role: "member" },
    }),
    false,
  );
  assert.equal(
    canEditConnection({
      ...connection,
      credential_source: { ...org, allowed: false },
    }),
    false,
  );
});

test("catalog lookup uses canonical catalog slug for repeated connections", () => {
  const wrong = {
    ...entry,
    slug: connection.slug,
    name: "Unrelated custom service",
  };
  assert.equal(catalogForConnection(connection, [wrong, entry]), entry);
  assert.equal(
    catalogForConnection({ ...connection, catalog_service_slug: null }, [
      wrong,
      entry,
    ]),
    undefined,
  );
});

test("managed OAuth is offered only when the declared credential mode permits it", () => {
  assert.equal(supportsManagedOAuth(entry), true);
  assert.equal(
    supportsManagedOAuth({ ...entry, credential_mode: "user" }),
    false,
  );
  assert.equal(
    supportsManagedOAuth({
      ...entry,
      has_platform_oauth_credentials: undefined,
    }),
    false,
  );
  assert.equal(
    supportsManagedOAuth({ ...entry, provider_config_id: null }),
    false,
  );
  assert.equal(
    supportsManagedOAuth({ ...entry, provider_type: "device_code" }),
    false,
  );
});

test("browser links reject credential-bearing URLs and executable/local schemes", () => {
  assert.equal(
    validateBrowserUrl("https://github.com/login/oauth/authorize?state=a"),
    "https://github.com/login/oauth/authorize?state=a",
  );
  for (const value of [
    "javascript:alert(1)",
    "file:///tmp/test",
    "http://example.com",
    "https://user:secret@example.com",
    "broken",
  ]) {
    assert.throws(() => validateBrowserUrl(value));
  }
});

test("catalog submission cannot invent a platform key or omit required credential", () => {
  const values = {
    label: "  Personal service  ",
    credential: "",
    endpointUrl: "",
    fields: {},
    platform: false,
  };
  assert.deepEqual(buildCatalogConnectionInput(entry, values), {
    service_slug: "api-github",
    label: "Personal service",
  });
  assert.throws(() =>
    buildCatalogConnectionInput(entry, { ...values, platform: true }),
  );
  assert.throws(() =>
    buildCatalogConnectionInput({ ...entry, provider_type: "api_key" }, values),
  );
  assert.deepEqual(
    buildCatalogConnectionInput(
      { ...entry, platform_key: { available: true } },
      { ...values, platform: true },
    ),
    {
      service_slug: "api-github",
      label: "Personal service",
      use_platform_key: true,
    },
  );
});

test("catalog and key list accept omitted optional metadata from skip-serializing backend responses", () => {
  assert.equal(
    catalogEntrySchema.safeParse({
      slug: "custom",
      name: "Custom",
      auth_method: "bearer",
      auth_key_name: "Authorization",
      service_type: "http",
      requires_credential: true,
      requires_gateway_url: false,
    }).success,
    true,
  );
  assert.equal(
    serviceConnectionSchema.safeParse({
      id: "custom-1",
      label: "Custom",
      slug: "custom",
      status: "active",
      is_active: true,
      auto_connected: false,
      credential_type: "bearer",
    }).success,
    true,
  );
});

test("connection and catalog schemas retain optional health and permission capabilities", () => {
  const parsedConnection = serviceConnectionSchema.parse({
    ...connection,
    expires_at: "2026-10-16T00:00:00Z",
    revocation: { revokes_grant: true },
  });
  assert.equal(parsedConnection.expires_at, "2026-10-16T00:00:00Z");
  assert.deepEqual(parsedConnection.revocation, { revokes_grant: true });
  const parsedEntry = catalogEntrySchema.parse({
    ...entry,
    supports_oauth_scopes: false,
    device_code_format: "openai",
    revocation: { revokes_grant: true },
    scope_catalog: [
      {
        scope: "repo",
        label: "Repositories",
        description: "Access repositories",
        required: true,
      },
      { scope: "read:user", label: "User profile" },
    ],
  });
  assert.equal(parsedEntry.supports_oauth_scopes, false);
  assert.equal(parsedEntry.device_code_format, "openai");
  assert.deepEqual(parsedEntry.revocation, { revokes_grant: true });
  assert.deepEqual(parsedEntry.scope_catalog, [
    {
      scope: "repo",
      label: "Repositories",
      description: "Access repositories",
      required: true,
    },
    { scope: "read:user", label: "User profile" },
  ]);
  assert.equal(serviceConnectionSchema.parse(connection).expires_at, undefined);
  assert.equal(
    catalogEntrySchema.parse(entry).supports_oauth_scopes,
    undefined,
  );
  assert.equal(
    serviceConnectionSchema.parse({ ...connection, expires_at: null })
      .expires_at,
    null,
  );
  assert.equal(
    catalogEntrySchema.parse({ ...entry, revocation: null }).revocation,
    null,
  );
});

test("token exchange credentials use only declared fields and validate gateway URLs", () => {
  const exchange = {
    ...entry,
    provider_type: "api_key",
    auth_method: "token_exchange",
    requires_gateway_url: true,
    token_exchange_credential_fields: [
      { name: "app_id", label: "App ID", secret: false },
      { name: "app_secret", label: "App secret", secret: true },
    ],
  };
  const values = {
    label: "App",
    credential: "",
    endpointUrl: "https://service.example",
    fields: { app_id: " id ", app_secret: " value ", ignored: "x" },
    platform: false,
  };
  assert.deepEqual(buildCatalogConnectionInput(exchange, values), {
    service_slug: entry.slug,
    label: "App",
    endpoint_url: "https://service.example/",
    credential: '{"app_id":"id","app_secret":"value"}',
  });
  assert.throws(
    () =>
      buildCatalogConnectionInput(exchange, {
        ...values,
        fields: { app_id: "id" },
      }),
    /App secret/,
  );
  assert.throws(() =>
    buildCatalogConnectionInput(exchange, {
      ...values,
      endpointUrl: "https://user:secret@service.example",
    }),
  );
});
