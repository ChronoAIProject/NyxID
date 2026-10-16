import assert from "node:assert/strict";
import test from "node:test";
import {
  agentCreateSchema,
  agentProfile,
  agentSchema,
  buildAgentPatch,
  changedGrants,
  grantOptions,
  grantSignature,
  profileHasConflict,
  type AssistantAgent,
} from "./agentContracts";
import type { ServiceConnection } from "../services/serviceContracts";

const agent: AssistantAgent = agentSchema.parse({
  id: "agent-1",
  kind: "specialist",
  name: "researcher",
  description: "Research reports",
  model: "model-a",
  status: "idle",
  owner_kind: "person",
  can_maintain: true,
  services: ["api-github"],
  account_read: false,
  guest_access: { "api-github": "read" },
});
const key: ServiceConnection = {
  id: "service-1",
  label: "GitHub",
  slug: "api-github",
  status: "active",
  is_active: true,
  auto_connected: false,
  credential_type: "oauth2",
  catalog_service_slug: "api-github",
  catalog_service_name: "GitHub",
  error_message: null,
  credential_source: { type: "personal" },
};

test("specialist creation validates handle and required purpose", () => {
  const values = {
    name: "researcher",
    description: "Research",
    display_name: "",
    persona: "",
    services: [],
    account_read: false,
  };
  assert.equal(agentCreateSchema.safeParse(values).success, true);
  for (const name of ["Researcher", "-researcher", "has space", "a".repeat(33)])
    assert.equal(
      agentCreateSchema.safeParse({ ...values, name }).success,
      false,
    );
  assert.equal(
    agentCreateSchema.safeParse({ ...values, description: " " }).success,
    false,
  );
  assert.equal(
    agentCreateSchema.safeParse({ ...values, display_name: "a".repeat(41) })
      .success,
    false,
  );
});

test("profile saves send only changed fields and retain explicit override clearing", () => {
  assert.deepEqual(
    buildAgentPatch(agent, { ...agentProfile(agent), display_name: "Luna" }),
    { display_name: "Luna" },
  );
  const overridden = { ...agent, access_mode_override: "autopilot" as const };
  assert.deepEqual(
    buildAgentPatch(overridden, {
      ...agentProfile(overridden),
      access_mode_override: null,
    }),
    { access_mode_override: null },
  );
  assert.deepEqual(buildAgentPatch(agent, agentProfile(agent)), {});
});

test("NyxBot does not send editable handle and allows empty notes", () => {
  const nyxbot = { ...agent, kind: "nyxbot" as const, name: "NyxBot" };
  assert.deepEqual(
    buildAgentPatch(nyxbot, {
      ...agentProfile(nyxbot),
      name: "other",
      description: "",
    }),
    { description: "" },
  );
});

test("profile conflict check blocks overwriting changed fields but permits unrelated changes", () => {
  const patch = { display_name: "Luna" };
  assert.equal(
    profileHasConflict(
      agent,
      { ...agent, description: "Updated purpose" },
      patch,
    ),
    false,
  );
  assert.equal(
    profileHasConflict(agent, { ...agent, display_name: "Other name" }, patch),
    true,
  );
});

test("org grant options are exact IDs in the same authorized owner scope", () => {
  const orgKey = {
    ...key,
    id: "org-service-id",
    credential_source: {
      type: "org" as const,
      org_id: "org-1",
      org_name: "Team",
      role: "admin" as const,
      allowed: true,
    },
  };
  const orgAgent = { ...agent, owner_kind: "org" as const, owner_id: "org-1" };
  assert.deepEqual(
    grantOptions(
      [
        key,
        orgKey,
        {
          ...orgKey,
          id: "other-org",
          credential_source: { ...orgKey.credential_source, org_id: "org-2" },
        },
        {
          ...orgKey,
          id: "denied",
          credential_source: { ...orgKey.credential_source, allowed: false },
        },
      ],
      orgAgent,
    ),
    [{ value: "org-service-id", label: "GitHub" }],
  );
  assert.deepEqual(
    grantOptions([key, { ...key, id: "disabled", is_active: false }]),
    [{ value: "api-github", label: "GitHub" }],
  );
});

test("permission signatures ignore ordering and detect account/guest authority changes", () => {
  const base = {
    ...agent,
    services: ["a", "b"],
    guest_access: { a: "read" as const, b: "use" as const },
  };
  assert.equal(
    grantSignature(base),
    grantSignature({
      ...base,
      services: ["b", "a"],
      guest_access: { b: "use", a: "read" },
    }),
  );
  assert.notEqual(
    grantSignature(base),
    grantSignature({ ...base, account_read: true }),
  );
  assert.notEqual(
    grantSignature(base),
    grantSignature({
      ...base,
      guest_access: { ...base.guest_access, a: "all" },
    }),
  );
});

test("permission submission preserves unchanged guest levels and omits machine/login fields", () => {
  assert.deepEqual(
    changedGrants(agent, ["api-github", "api-github"], true, {
      "api-github": "read",
    }),
    { services: ["api-github"], account_read: true },
  );
  assert.deepEqual(
    changedGrants(agent, ["api-github", "api-drive"], false, {
      "api-github": "all",
      "api-drive": "use",
    }),
    {
      services: ["api-github", "api-drive"],
      account_read: false,
      guest_access: { "api-github": "all" },
    },
  );
});
