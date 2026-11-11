import assert from "node:assert/strict";
import test from "node:test";
import {
  canManageProviderPermissions,
  describeConnection,
  isOAuthConnection,
  reconnectScopeOverride,
} from "./connectionPresentation";
import type { CatalogEntry, ServiceConnection } from "./serviceContracts";

const connection: ServiceConnection = {
  id: "connection-1",
  label: "Personal GitHub",
  slug: "api-github-personal",
  status: "active",
  is_active: true,
  auto_connected: false,
  credential_type: "oauth2",
  credential_source: { type: "personal" },
};

const entry: CatalogEntry = {
  slug: "api-github",
  name: "GitHub",
  auth_method: "bearer",
  auth_key_name: "Authorization",
  service_type: "http",
  provider_type: "oauth2",
  requires_credential: true,
  requires_gateway_url: false,
};

test("enabled and paused connections retain the same credential health", () => {
  const active = describeConnection(connection);
  assert.deepEqual(
    describeConnection({ ...connection, is_active: false }),
    active,
  );
  assert.equal(active.tone, "success");
  assert.equal(active.recovery, undefined);
  assert.match(active.description, /stored credential/);
  assert.doesNotMatch(active.description, /verified|working|tested/i);
});

test("a missing credential leads the status even when lifecycle says active", () => {
  const result = describeConnection({
    ...connection,
    credential_missing: true,
    connection_status: "active",
  });
  assert.equal(result.label, "Credential missing");
  assert.equal(result.tone, "error");
  assert.equal(result.recovery, "reconnect");
});

test("credential failures cannot be masked by active OAuth metadata", () => {
  for (const status of ["failed", "refresh_failed", "revoked"] as const) {
    const result = describeConnection({
      ...connection,
      status,
      connection_status: "active",
    });
    assert.equal(result.tone, "error", status);
    assert.equal(
      result.recovery,
      status === "revoked" ? undefined : "reconnect",
      status,
    );
  }
});

test("unfinished authentication offers continue while failures offer reconnect", () => {
  const pending = describeConnection({
    ...connection,
    status: "pending_auth",
  });
  assert.equal(pending.recovery, "continue");
  assert.equal(pending.tone, "warning");
  assert.match(pending.description, /has not returned a credential/);
  const failed = describeConnection({ ...connection, status: "failed" });
  assert.match(failed.description, /did not complete|rejected/);
  const renewal = describeConnection({
    ...connection,
    status: "refresh_failed",
  });
  assert.match(renewal.description, /renew/);
});

test("expired lifecycle and OAuth metadata warn without starting reauthorization", () => {
  const expired = describeConnection({ ...connection, status: "expired" });
  assert.equal(expired.tone, "warning");
  assert.equal(expired.recovery, undefined);
  assert.match(expired.description, /automatically/);
  assert.deepEqual(
    describeConnection({ ...connection, connection_status: "expired" }),
    expired,
  );
  assert.equal(
    describeConnection({
      ...connection,
      status: "failed",
      connection_status: "expired",
    }).label,
    "Authorization failed",
  );
});

test("no-auth, platform and node credentials explain their source honestly", () => {
  const noAuth = describeConnection({ ...connection, credential_type: "none" });
  assert.equal(noAuth.label, "No authentication required");
  assert.equal(noAuth.recovery, undefined);
  const platform = describeConnection({
    ...connection,
    credential_type: "none",
    credential_binding: "platform",
  });
  assert.equal(platform.label, "Platform credential");
  assert.equal(platform.tone, "muted");
  const node = describeConnection({
    ...connection,
    credential_type: "node_managed",
  });
  assert.equal(node.label, "Managed on your node");
  assert.equal(node.tone, "muted");
  assert.equal(node.recovery, undefined);
});

test("unknown states remain readable without inventing a recovery operation", () => {
  const unknown = describeConnection({
    ...connection,
    status: "provider_under_review",
    connection_status: "active",
  });
  assert.equal(unknown.label, "provider under review");
  assert.equal(unknown.tone, "warning");
  assert.equal(unknown.recovery, undefined);
  assert.equal(
    describeConnection({ ...connection, status: "" }).label,
    "Unknown credential status",
  );
});

test("OAuth recognition includes provider metadata for missing and device credentials", () => {
  assert.equal(isOAuthConnection(connection), true);
  const missing = { ...connection, credential_type: "none" };
  assert.equal(isOAuthConnection(missing), false);
  assert.equal(isOAuthConnection(missing, entry), true);
  assert.equal(
    isOAuthConnection(missing, { ...entry, provider_type: "device_code" }),
    true,
  );
  assert.equal(
    isOAuthConnection(missing, { ...entry, provider_type: "api_key" }),
    false,
  );
});

test("provider permission editing requires a healthy editable OAuth connection", () => {
  assert.equal(canManageProviderPermissions(connection, entry), true);
  // Pausing service use does not remove an existing provider authorization.
  assert.equal(
    canManageProviderPermissions({ ...connection, is_active: false }, entry),
    true,
  );
  assert.equal(canManageProviderPermissions(connection), false);
  for (const override of [
    { status: "pending_auth" },
    { status: "failed" },
    { status: "refresh_failed" },
    { status: "revoked" },
    { status: "expired" },
    { credential_missing: true },
    { credential_binding: "platform" as const },
    { auto_connected: true },
    { can_edit_configuration: false },
    { credential_source: undefined },
  ]) {
    assert.equal(
      canManageProviderPermissions({ ...connection, ...override }, entry),
      false,
      JSON.stringify(override),
    );
  }
});

test("provider capability and organization role gates match Web permissions", () => {
  assert.equal(
    canManageProviderPermissions(connection, {
      ...entry,
      supports_oauth_scopes: false,
    }),
    false,
  );
  assert.equal(
    canManageProviderPermissions(connection, {
      ...entry,
      provider_type: "device_code",
      device_code_format: "openai",
    }),
    false,
  );
  const org = {
    type: "org" as const,
    org_id: "org-1",
    org_name: "Company",
    allowed: true,
  };
  for (const role of ["member", "viewer"] as const) {
    assert.equal(
      canManageProviderPermissions(
        { ...connection, credential_source: { ...org, role } },
        entry,
      ),
      false,
      role,
    );
  }
  assert.equal(
    canManageProviderPermissions(
      { ...connection, credential_source: { ...org, role: "admin" } },
      entry,
    ),
    true,
  );
  assert.equal(
    canManageProviderPermissions(
      { ...connection, credential_type: "api_key" },
      { ...entry, provider_type: "api_key" },
    ),
    false,
  );
});

test("repair preserves custom granted scopes and adds only mandatory catalog entries", () => {
  assert.deepEqual(
    reconnectScopeOverride(
      {
        ...connection,
        granted_scopes: ["repo", "custom:assistant-scope", "repo"],
      },
      {
        ...entry,
        scope_catalog: [
          { scope: "read:user", label: "Profile", required: true },
          { scope: "repo", label: "Repositories", required: true },
          { scope: "workflow", label: "Workflows" },
        ],
      },
    ),
    ["repo", "custom:assistant-scope", "read:user"],
  );
});

test("repair retains an explicit empty grant rather than using provider defaults", () => {
  assert.deepEqual(
    reconnectScopeOverride({ ...connection, granted_scopes: [] }, entry),
    [],
  );
  assert.deepEqual(
    reconnectScopeOverride(
      { ...connection, granted_scopes: [] },
      {
        ...entry,
        scope_catalog: [{ scope: "openid", label: "Identity", required: true }],
      },
    ),
    ["openid"],
  );
});

test("unsupported scope protocols and absent saved grants leave defaults untouched", () => {
  const saved = { ...connection, granted_scopes: ["repo"] };
  assert.equal(
    reconnectScopeOverride(saved, { ...entry, supports_oauth_scopes: false }),
    undefined,
  );
  assert.equal(
    reconnectScopeOverride(saved, {
      ...entry,
      provider_type: "device_code",
      device_code_format: "openai",
    }),
    undefined,
  );
  assert.equal(reconnectScopeOverride(connection, entry), undefined);
  assert.equal(
    reconnectScopeOverride({ ...connection, granted_scopes: null }, entry),
    undefined,
  );
});
