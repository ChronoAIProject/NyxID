import assert from "node:assert/strict";
import test from "node:test";
import {
  agentCreateSchema,
  agentProfile,
  agentSchema,
  buildAgentPatch,
  changedGrants,
  grantOptions,
  grantServiceSchema,
  grantSignature,
  profileHasConflict,
  type AssistantAgent,
  type GrantService,
} from "./agentContracts";

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
const key: GrantService = {
  id: "service-1",
  label: "GitHub",
  slug: "api-github",
  executable: true,
  catalog_slug: "api-github",
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

test("grant options use the server scoped inventory with exact org IDs", () => {
  const rows = [
    { id: "one", slug: "calendar", label: "Calendar", executable: false },
    { id: "two", slug: "calendar", label: "Other Calendar", executable: true },
  ];
  assert.deepEqual(grantOptions(rows, { ...agent, owner_kind: "org" }), [
    { value: "one", label: "Calendar", iconSlug: "calendar", iconUrl: undefined },
    { value: "two", label: "Other Calendar", iconSlug: "calendar", iconUrl: undefined },
  ]);
  assert.deepEqual(grantOptions(rows), [{ value: "calendar", label: "Calendar", iconSlug: "calendar", iconUrl: undefined }]);
  assert.equal(grantOptions([{ id: "public", slug: "chrono-llm-public", label: "Chrono LLM (public)", executable: true }])[0]?.value, "chrono-llm-public");
});

test("specialist tool grants exclude NyxBot Voice by catalog identity and legacy slug", () => {
  const voice = {
    ...key,
    id: "voice-connection",
    label: "NyxBot Voice",
    slug: "my-bot-voice",
    catalog_slug: "nyxbot-voice",
  };
  assert.deepEqual(
    grantOptions([
      voice,
      {
        ...voice,
        id: "legacy-voice",
        slug: "nyxbot-voice",
        catalog_slug: undefined,
      },
      key,
    ]).map((option) => option.value),
    ["api-github"],
  );
  assert.deepEqual(
    grantOptions(
      [voice, key],
      { ...agent, owner_kind: "org", owner_id: "org-1" },
    ).map((option) => option.value),
    [key.id],
  );
});

test("other server-approved connections remain grantable even when not executable", () => {
  const connections = [
    key,
    {
      ...key,
      id: "public",
      slug: "public-tools",
    },
    {
      ...key,
      id: "offline",
      slug: "offline-tools",
      executable: false,
    },
    {
      ...key,
      id: "similarly-named",
      slug: "nyxbot-voice-helper",
      catalog_slug: undefined,
    },
    {
      ...key,
      id: "different-catalog",
      slug: "nyxbot-voice",
      catalog_slug: "other-service",
    },
  ];
  assert.deepEqual(
    grantOptions(connections).map((option) => option.value),
    connections.map((connection) => connection.slug),
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

test("agent create/profile writes strip legacy model selection fields", () => {
  const values = {
    ...agentProfile(agent),
    model: "nyxagent/research",
    services: [],
    account_read: false,
  };
  assert.equal("model" in agentCreateSchema.parse(values), false);
  assert.deepEqual(buildAgentPatch(agent, values), {});
});


test("grant options preserve the catalog glyph and icon for a differently named connection", () => {
  assert.deepEqual(grantOptions([grantServiceSchema.parse({
    id: "github-second", slug: "api-github-2", label: "Second GitHub", executable: true,
    catalog_slug: "api-github", icon_url: "https://example.test/catalog-github.svg",
  })]), [{
    value: "api-github-2", label: "Second GitHub", iconSlug: "api-github",
    iconUrl: "https://example.test/catalog-github.svg",
  }]);
});

test("grant options preserve custom icons and accept older inventory rows without icon metadata", () => {
  assert.deepEqual(grantOptions([grantServiceSchema.parse({
    id: "custom", slug: "renamed-account", label: "Custom", executable: true,
    catalog_slug: "api-github", icon_url: "https://example.test/my-icon.svg",
  }), grantServiceSchema.parse({
    id: "older", slug: "custom-api", label: "Older", executable: true,
  })]), [{
    value: "renamed-account", label: "Custom", iconSlug: "api-github",
    iconUrl: "https://example.test/my-icon.svg",
  }, {
    value: "custom-api", label: "Older", iconSlug: "custom-api", iconUrl: undefined,
  }]);
});
