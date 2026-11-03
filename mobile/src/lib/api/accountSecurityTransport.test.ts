import assert from "node:assert/strict";
import test from "node:test";
import {
  createAccountSecurityApi,
  changePasswordSchema,
} from "./accountSecurityTransport";
import type { RequestOptions } from "./authenticatedTransport";

test("MFA disable sends a required authenticator/recovery code and only supplied passwords without invalidating a bad-code session", async () => {
  const calls: { path: string; options?: RequestOptions }[] = [];
  const api = createAccountSecurityApi(async (path, options) => {
    calls.push({ path, options });
    return {};
  });
  await api.disableMfa({ code: " 123456 ", password: "current password" });
  await api.disableMfa({ code: "recovery-code" });
  assert.deepEqual(
    calls.map((call) => call.options?.body),
    [
      { code: "123456", password: "current password" },
      { code: "recovery-code" },
    ],
  );
  for (const call of calls) {
    assert.equal(call.path, "/auth/mfa/disable");
    assert.equal(call.options?.retryOnAuthFailure, false);
    assert.equal(call.options?.invalidateSessionOnAuthFailure, false);
  }
  assert.throws(() => api.disableMfa({ code: "  " }));
  assert.equal(calls.length, 2);
});

test("all security writes preserve lifecycle guard and cancellation signal", async () => {
  const calls: RequestOptions[] = [];
  const authorityGuard = () => {};
  const controller = new AbortController();
  const guard = { authorityGuard, signal: controller.signal };
  const api = createAccountSecurityApi(async (path, options) => {
    calls.push(options!);
    if (path === "/users/me/security")
      return { skip_mfa_for_approved_sign_ins: false };
    if (path === "/orgs")
      return {
        id: "org",
        slug: "team",
        display_name: "Team",
        your_role: "admin",
      };
    if (path.startsWith("/orgs/join/"))
      return { org_id: "org", role: "member" };
    return {};
  });
  await api.disableMfa({ code: "123456" }, guard);
  await api.changePassword(
    "owner",
    {
      current_password: "old",
      new_password: "new password",
      confirmation: "new password",
    },
    guard,
  );
  await api.saveSecurityPreferences(
    "owner",
    { skip_mfa_for_approved_sign_ins: false },
    guard,
  );
  await api.revokeConsent("owner", "client", guard);
  await api.revokeBrokerBinding("owner", "hash", guard);
  await api.createOrganization("owner", "Team", guard);
  await api.joinOrganization("owner", "nonce", guard);
  assert.equal(calls.length, 7);
  for (const options of calls) {
    assert.equal(options.authorityGuard, authorityGuard);
    assert.equal(options.signal, controller.signal);
  }
  assert.ok(
    calls.slice(1).every((options) => options.expectedAccountId === "owner"),
  );
  assert.ok(
    calls
      .slice(0, 3)
      .every((options) => options.invalidateSessionOnAuthFailure === false),
  );
});

test("password validation preserves whitespace/UTF-8 limits, strips confirmation and fences to the current account", async () => {
  const calls: RequestOptions[] = [];
  const api = createAccountSecurityApi(async (_path, options) => {
    calls.push(options!);
    return {};
  });
  assert.equal(
    changePasswordSchema.safeParse({
      current_password: "old",
      new_password: "界".repeat(43),
      confirmation: "界".repeat(43),
    }).success,
    false,
  );
  assert.equal(
    changePasswordSchema.safeParse({
      current_password: "old",
      new_password: "12345678",
      confirmation: "different",
    }).success,
    false,
  );
  await api.changePassword("owner", {
    current_password: " old ",
    new_password: " new password ",
    confirmation: " new password ",
  });
  assert.deepEqual(calls[0]?.body, {
    current_password: " old ",
    new_password: " new password ",
  });
  assert.equal(calls[0]?.expectedAccountId, "owner");
  assert.equal(calls[0]?.invalidateSessionOnAuthFailure, false);
});

test("authorization lists are distinct from approval grants and revoke encoded identities under the shown account", async () => {
  const calls: { path: string; options?: RequestOptions }[] = [];
  const api = createAccountSecurityApi(async (path, options) => {
    calls.push({ path, options });
    return path.endsWith("consents")
      ? { consents: [] }
      : path.endsWith("broker-bindings")
        ? { bindings: [] }
        : {};
  });
  assert.deepEqual(await api.consents("owner"), []);
  assert.deepEqual(await api.brokerBindings("owner"), []);
  await api.revokeConsent("owner", "client/id");
  await api.revokeBrokerBinding("owner", "hash/id");
  assert.deepEqual(
    calls.map((call) => call.path),
    [
      "/users/me/consents",
      "/users/me/broker-bindings",
      "/users/me/consents/client%2Fid",
      "/users/me/broker-bindings/hash%2Fid",
    ],
  );
  assert.ok(calls.every((call) => call.options?.expectedAccountId === "owner"));
  assert.ok(calls.slice(2).every((call) => call.options?.method === "DELETE"));
});

test("security preferences and org create/join follow the actual server contracts, including owner role", async () => {
  const calls: { path: string; options?: RequestOptions }[] = [];
  const org = {
    id: "organization",
    slug: "team",
    display_name: "Team",
    your_role: "owner",
  };
  const api = createAccountSecurityApi(async (path, options) => {
    calls.push({ path, options });
    if (path === "/users/me/security")
      return { skip_mfa_for_approved_sign_ins: false };
    if (path.startsWith("/orgs/join/"))
      return { org_id: org.id, role: "member" };
    return options?.method === "POST" ? org : { orgs: [org] };
  });
  assert.deepEqual(
    await api.saveSecurityPreferences("owner", {
      skip_mfa_for_approved_sign_ins: false,
    }),
    { skip_mfa_for_approved_sign_ins: false },
  );
  assert.equal((await api.organizations("owner"))[0]?.your_role, "owner");
  await api.createOrganization("owner", " Team ");
  assert.deepEqual(calls[2]?.options?.body, { display_name: "Team" });
  await api.joinOrganization("owner", "nonce/slash");
  assert.equal(calls[3]?.path, "/orgs/join/nonce%2Fslash");
  assert.ok(calls.every((call) => call.options?.expectedAccountId === "owner"));
  await assert.rejects(api.createOrganization("owner", " "));
});

test("malformed authorization data fails closed instead of silently showing no access", async () => {
  const api = createAccountSecurityApi(async () => ({
    consents: [{ client_id: "x" }],
  }));
  await assert.rejects(api.consents("owner"));
});

test("nonempty consent and broker fixtures preserve service access and nullable identity metadata", async () => {
  const consent = {
    id: "consent",
    client_id: "client",
    client_name: "Desktop app",
    scopes: "openid account:read",
    allow_all_services: false,
    allowed_service_ids: ["connection"],
    legacy_unrestricted: false,
    allowed_services: [
      {
        id: "connection",
        label: "Calendar",
        slug: null,
        catalog_service_name: null,
        deleted: false,
      },
    ],
    granted_at: "2026-11-03T00:00:00Z",
    expires_at: null,
  };
  const binding = {
    binding_hash: "hash",
    client_id: "client",
    client_name: null,
    scopes: ["read"],
    external_subject: { platform: "slack", external_user_id: "external-user" },
    created_at: "2026-11-03T00:00:00Z",
    last_used_at: null,
  };
  const api = createAccountSecurityApi(async (path) =>
    path.endsWith("consents")
      ? { consents: [consent] }
      : {
          bindings: [
            binding,
            { ...binding, binding_hash: "second", external_subject: null },
          ],
        },
  );
  const [parsedConsent] = await api.consents("owner");
  assert.ok(parsedConsent);
  assert.equal(parsedConsent.allowed_services[0]?.label, "Calendar");
  assert.equal(parsedConsent.expires_at, null);
  assert.equal(parsedConsent.allow_all_services, false);
  const bindings = await api.brokerBindings("owner");
  assert.deepEqual(bindings[0], binding);
  assert.equal(bindings[1]?.external_subject, null);
});
