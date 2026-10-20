import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL } from "node:url";
import test from "node:test";
import { ACCOUNT_DELETION_DISCLOSURE, ACCOUNT_DELETION_POLICY_VERSION } from "./accountDeletionPolicy";

test("native, web and CLI disclose the identical versioned account deletion policy", () => {
  assert.equal(ACCOUNT_DELETION_POLICY_VERSION, "machine-data-provider-revocation-v2");
  const web = readFileSync(new URL("../../../frontend/src/lib/account-deletion-policy.ts", import.meta.url), "utf8");
  const cli = readFileSync(new URL("../../../cli/src/commands/profile.rs", import.meta.url), "utf8");
  for (const source of [web, cli]) {
    assert.ok(source.includes(`"${ACCOUNT_DELETION_POLICY_VERSION}"`));
    const literal = source.match(/ACCOUNT_DELETION_DISCLOSURE(?:: &str)? = ("(?:[^"\\]|\\.)*");/);
    const encoded = literal?.[1];
    assert.ok(encoded);
    assert.equal(JSON.parse(encoded), ACCOUNT_DELETION_DISCLOSURE);
  }
  assert.match(ACCOUNT_DELETION_DISCLOSURE, /every connected provider authorization/);
  assert.match(ACCOUNT_DELETION_DISCLOSURE, /Unsupported or unconfirmed revocations/);
});
