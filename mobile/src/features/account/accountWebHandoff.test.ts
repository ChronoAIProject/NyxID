import assert from "node:assert/strict";
import test from "node:test";
import { accountWebUrl, ORGANIZATION_WEB_ACTIONS } from "./accountWebHandoff";
test("account handoffs stay on the configured website and use valid per-action organization tabs", () => {
  assert.equal(
    accountWebUrl("https://nyxid.example/base", "/register"),
    "https://nyxid.example/register",
  );
  assert.equal(
    accountWebUrl("https://nyxid.example", "/orgs/id?tab=role-permissions"),
    "https://nyxid.example/orgs/id?tab=role-permissions",
  );
  for (const base of [
    "",
    "javascript:alert(1)",
    "https://name:pass@example.test",
  ])
    assert.throws(() => accountWebUrl(base, "/register"));
  assert.throws(() =>
    accountWebUrl("https://nyxid.example", "https://other.example/register"),
  );
  assert.deepEqual(
    ORGANIZATION_WEB_ACTIONS.map((action) => action.tab),
    ["members", "invites", "approvals", "role-permissions", "settings"],
  );
  assert.deepEqual(
    ORGANIZATION_WEB_ACTIONS.filter((action) => !action.adminOnly).map(
      (action) => action.tab,
    ),
    ["members"],
  );
});
