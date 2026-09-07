import assert from "node:assert/strict";
import { test } from "node:test";
import { normalizeAgentKeyExpiry } from "./agentKeyExpiry";
import { agentKeyLoginError } from "./agentKeyLoginModel";
import { agentKeyApproveSchema, newAgentKeySchema } from "../../lib/api/agentKeyLoginSchema";

test("expiry accepts valid dates at the end of the UTC day and RFC 3339 offsets", () => {
  assert.equal(normalizeAgentKeyExpiry(" 2028-02-29 "), "2028-02-29T23:59:59Z");
  assert.equal(normalizeAgentKeyExpiry("2099-01-01T12:30:59+08:00"), "2099-01-01T12:30:59+08:00");
  assert.equal(normalizeAgentKeyExpiry("2099-01-01t12:30:59.123z"), "2099-01-01T12:30:59.123Z");
  for (const value of ["9999-12-31T23:59:59-23:59", "2099-01-01T12:30:59.123456789Z"]) {
    assert.equal(normalizeAgentKeyExpiry(value), value);
  }
});

test("expiry rejects free text, impossible dates and timestamps without seconds or timezone", () => {
  for (const value of ["", "tomorrow", "01/01/2099", "2099-02-29", "2099-04-31", "2099-01-01T12:00:00", "2099-01-01T12:00Z", "2099-01-01T25:00:00Z", "2099-01-01T12:00:00+25:00"]) {
    assert.throws(() => normalizeAgentKeyExpiry(value), /YYYY-MM-DD or an RFC 3339 timestamp/);
  }
});

test("both key and credential expiry are normalized before approval", () => {
  const selection = newAgentKeySchema.parse({ kind: "new", name: "CLI", scopes: "read proxy", expires_at: "2099-01-02" });
  const approval = agentKeyApproveSchema.parse({ user_code: "ABCD-EFGH", selection, credential_expires_at: "2099-01-01" });
  assert.equal(approval.selection.kind === "new" && approval.selection.expires_at, "2099-01-02T23:59:59Z");
  assert.equal(approval.credential_expires_at, "2099-01-01T23:59:59Z");
  for (const expires_at of ["bad", "2099-02-30", "2000-01-01"]) {
    assert.equal(newAgentKeySchema.safeParse({ ...selection, expires_at }).success, false);
    assert.equal(agentKeyApproveSchema.safeParse({ user_code: "ABCD-EFGH", selection, credential_expires_at: expires_at }).success, false);
  }
});

test("invalid credential expiry displays one clear inline issue", () => {
  const result = agentKeyApproveSchema.safeParse({ user_code: "ABCD-EFGH", selection: { kind: "existing", api_key_id: "key" }, credential_expires_at: "tomorrow" });
  assert.equal(result.success, false);
  if (!result.success) {
    assert.equal(agentKeyLoginError(result.error), "Enter an expiry as YYYY-MM-DD or an RFC 3339 timestamp with a timezone.");
  }
});
