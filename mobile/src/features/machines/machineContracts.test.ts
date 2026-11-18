import assert from "node:assert/strict";
import test from "node:test";
import {
  machineNodeSchema,
  manageableMachines,
  machineWebPath,
} from "./machineContracts";

const node = (id: string, kind = "user", owner = "me", source?: string) =>
  machineNodeSchema.parse({
    id,
    name: id,
    owner: { kind, id: owner, display_name: owner },
    status: "active",
    is_connected: true,
    metadata: { provisioning_source: source },
    machine: {
      version: 2,
      shell: true,
      files: true,
      computer: false,
      os: "linux",
      arch: "arm64",
      roots: ["/workspace"],
      browser_isolated: true,
      computer_ready: false,
      saved_login_ready: false,
    },
  });
test("inventory includes personal and org Admin only and excludes hosted by either marker", () => {
  const nodes = [
    node("personal"),
    node("other", "user", "other"),
    node("admin", "org", "oa"),
    node("member", "org", "om"),
    node("viewer", "org", "ov"),
    node("managed", "user", "me", "remote_machine"),
    node("hosted"),
    node("no-machine"),
  ];
  nodes.at(-1)!.machine = null;
  const orgs = [
    { id: "oa", your_role: "admin" },
    { id: "om", your_role: "member" },
    { id: "ov", your_role: "viewer" },
  ];
  assert.deepEqual(
    manageableMachines(nodes, orgs, new Set(["hosted"]), "me").map(
      (row) => row.id,
    ),
    ["personal", "admin"],
  );
  assert.deepEqual(
    manageableMachines(nodes.slice(2, 3), [], new Set(), "me"),
    [],
    "removed membership",
  );
  assert.deepEqual(
    manageableMachines(nodes.slice(0, 1), [], new Set(), "other"),
    [],
    "transferred machine",
  );
});
test("canonical machine handoffs encode IDs and select saved logins", () => {
  assert.equal(machineWebPath(), "/assistant/machines");
  assert.equal(
    machineWebPath("a&tab=logins"),
    "/assistant/machines?machine=a%26tab%3Dlogins",
  );
  assert.equal(
    machineWebPath(undefined, "logins"),
    "/assistant/machines?tab=logins",
  );
});

import {
  machinePolicyInput,
  savedLoginSchema,
  machineUpdateResultSchema,
  updateResultMessage,
} from "./machineContracts";
test("single-user saved-login opt-in requires acknowledgement; isolated browser does not", () => {
  const machine = node("non-isolated");
  machine.machine!.browser_isolated = false;
  const input = {
    machine_confirm: "changes" as const,
    allow_single_user_saved_logins: true,
    acknowledge_single_user_risk: false,
  };
  assert.throws(() => machinePolicyInput(machine, input), /Acknowledge/);
  assert.equal(
    machinePolicyInput(machine, {
      ...input,
      acknowledge_single_user_risk: true,
    }).allow_single_user_saved_logins,
    true,
  );
  machine.machine!.browser_isolated = true;
  assert.equal(
    machinePolicyInput(machine, input).acknowledge_single_user_risk,
    false,
  );
  assert.throws(() =>
    machinePolicyInput(machine, { ...input, shell: true } as typeof input),
  );
});
test("saved-login metadata strips unexpected secret fields", () => {
  const result = savedLoginSchema.parse({
    id: "login",
    owner_id: "me",
    label: "Work",
    allowed_origins: ["https://example.test"],
    has_password: true,
    has_totp: true,
    confirm_each_sign_in: true,
    created_at: "now",
    updated_at: "now",
    password: "never-cache",
    totp_secret: "never-cache",
  });
  assert.equal("password" in result, false);
  assert.equal("totp_secret" in result, false);
});
test("HTTP success with queued, manual or failed update remains honest", () => {
  assert.match(
    updateResultMessage(machineUpdateResultSchema.parse({ status: "queued" })),
    /not completion/,
  );
  assert.match(
    updateResultMessage(
      machineUpdateResultSchema.parse({ status: "manual_step" }),
    ),
    /manual step/,
  );
  assert.match(
    updateResultMessage(
      machineUpdateResultSchema.parse({ status: "failed", reason: "busy" }),
    ),
    /failed: busy/,
  );
  assert.throws(() => machineUpdateResultSchema.parse({ status: "completed" }));
});
