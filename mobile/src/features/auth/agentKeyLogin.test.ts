import assert from "node:assert/strict";
import test from "node:test";
import { extractLoginRequestFromQr } from "./deviceUserCode";
import {
  agentKeyApproveSchema,
  newAgentKeySchema,
  agentKeyOptionsSchema,
  agentKeyPreviewSchema,
} from "../../lib/api/agentKeyLoginSchema";
import {
  defaultNewAgentKey,
  issuanceNotice,
  newKeySummary,
  permissionRows,
} from "./agentKeyLoginModel";

const policy = {
  appScheme: "nyxid",
  webOrigins: ["https://nyxid.example"],
  allowHttp: false,
};

test("Agent Key QR parser accepts trusted web and app links without returning unrelated parameters", () => {
  for (const prefix of [
    "https://nyxid.example/login/agent-key",
    "nyxid://login/agent-key",
    "nyxid:///login/agent-key/",
  ]) {
    assert.deepEqual(
      extractLoginRequestFromQr(
        `${prefix}?user_code=abcd-efgh&credential=ignored`,
        policy,
      ),
      { kind: "agent-key", userCode: "ABCDEFGH" },
    );
  }
  assert.deepEqual(
    extractLoginRequestFromQr(
      "nyxid://login/device?user_code=abcd-efgh",
      policy,
    ),
    { kind: "device", userCode: "ABCDEFGH" },
  );
});

test("Agent Key QR parser rejects untrusted origins, duplicates and malformed URLs", () => {
  for (const input of [
    "https://evil.example/login/agent-key?user_code=ABCDEFGH",
    "http://nyxid.example/login/agent-key?user_code=ABCDEFGH",
    "https://nyxid.example@evil.example/login/agent-key?user_code=ABCDEFGH",
    "nyxid://login/agent-key?user_code=ABCDEFGH&user_code=ABCDEFGH",
    "nyxid://login/agent-key?user_code=%ZZ",
    "nyxid://login/agent-key?user_code=short",
  ])
    assert.equal(extractLoginRequestFromQr(input, policy), null);
});

test("new key defaults and confirmation keep service and node grants limited", () => {
  const draft = defaultNewAgentKey(Date.now());
  assert.equal(draft.scopes, "read proxy");
  assert.equal(draft.allow_all_services, false);
  assert.equal(draft.allow_all_nodes, false);
  assert.deepEqual(draft.allowed_service_ids, []);
  assert.deepEqual(draft.allowed_node_ids, []);
  assert.ok(Date.parse(draft.expires_at!) > Date.now() + 89 * 86400000);
  const options = agentKeyOptionsSchema.parse({
    keys: [],
    services: [{ id: "svc", name: "Service name", owner_id: "org" }],
    nodes: [],
    orgs: [{ id: "org", name: "Organization", owner_id: "org" }],
  });
  const summary = newKeySummary(
    { ...draft, target_org_id: "org", allowed_service_ids: ["svc"] },
    options,
  );
  assert.equal(summary.owner_type, "org");
  assert.equal(summary.owner_name, "Organization");
  assert.equal(
    permissionRows(summary).find((row) => row.label === "Services")?.value,
    "Service name",
  );
  assert.equal(
    permissionRows({ ...summary, scopes: "admin" }).find(
      (row) => row.label === "Effective permissions",
    )?.value,
    "read, write",
  );
  assert.equal(
    permissionRows({ ...summary, allow_all_nodes: true }).find(
      (row) => row.label === "Nodes",
    )?.warning,
    true,
  );
  assert.match(
    issuanceNotice(true),
    /existing secret and other consumers are unaffected/,
  );
});

test("approval schema normalizes only valid codes and rejects broader or expired inputs", () => {
  const selection = defaultNewAgentKey();
  assert.equal(
    agentKeyApproveSchema.parse({ user_code: "abcd-efgh", selection })
      .user_code,
    "ABCDEFGH",
  );
  for (const invalid of [
    { scopes: "root" },
    { name: " " },
    { allow_all_services: true, allowed_service_ids: ["svc"] },
    { expires_at: "2000-01-01" },
    { rate_limit_burst: 0 },
  ]) {
    assert.equal(
      agentKeyApproveSchema.safeParse({
        user_code: "ABCDEFGH",
        selection: { ...selection, ...invalid },
      }).success,
      false,
    );
  }
  assert.equal(
    agentKeyApproveSchema.safeParse({ user_code: "short", selection }).success,
    false,
  );
});

test("public preview strips credential material and sanitizes requester fields", () => {
  const parsed = agentKeyPreviewSchema.parse({
    client_label: "device\u0000",
    requested_profile: "home",
    interval: 5,
    status: "pending",
    initiated_at: "2099-01-01T00:00:00Z",
    expires_at: "2099-01-01T00:10:00Z",
    credential: "not-returned",
  });
  assert.equal(parsed.client_label, "device");
  assert.equal(parsed.client_ip_attribution, "unavailable");
  assert.equal("credential" in parsed, false);
});

test("platform scope is preserved and described as including future services", () => {
  const draft = {
    ...defaultNewAgentKey(),
    allow_auto_connected_services: true,
  };
  const parsed = newAgentKeySchema.parse(draft);
  assert.equal(parsed.allow_auto_connected_services, true);
  const summary = newKeySummary(parsed, {
    keys: [],
    services: [
      {
        id: "platform",
        name: "Search",
        owner_id: "person",
        auto_connected: true,
      },
    ],
    nodes: [],
    personal_owner_id: "person",
    orgs: [],
  });
  assert.equal(summary.allowed_services[0]?.auto_connected, true);
  assert.match(
    permissionRows(summary).find((row) => row.label === "Services")!.value,
    /including future additions/,
  );
});

test("platform summary follows key ownership while retaining explicit org grants", () => {
  const options = {
    keys: [],
    services: [
      {
        id: "personal-platform",
        name: "Search",
        owner_id: "person",
        auto_connected: true,
      },
      {
        id: "org-platform",
        name: "Org search",
        owner_id: "org",
        auto_connected: true,
      },
    ],
    nodes: [],
    personal_owner_id: "person",
    orgs: [{ id: "org", name: "Org", owner_id: "org" }],
  };
  const input = {
    ...defaultNewAgentKey(),
    allow_auto_connected_services: true,
  };
  assert.deepEqual(
    newKeySummary(input, options).allowed_services.map((item) => item.id),
    ["personal-platform"],
  );
  assert.deepEqual(
    newKeySummary(
      { ...input, target_org_id: "org" },
      options,
    ).allowed_services.map((item) => item.id),
    ["org-platform"],
  );
  assert.equal(
    newKeySummary({ ...input, allowed_service_ids: ["org-platform"] }, options)
      .allowed_services.length,
    2,
  );
});
