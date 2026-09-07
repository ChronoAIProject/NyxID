import assert from "node:assert/strict";
import test from "node:test";
import { extractLoginRequestFromQr } from "./deviceUserCode";
import {
  agentKeyApproveSchema,
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
