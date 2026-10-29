import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import * as contracts from "../../features/machines/machineContracts";

function setup(pages: unknown[]) {
  const paths: string[] = [];
  const harness = hookHarness({
    "../auth/authEpoch": { getAuthEpoch: () => 1 },
    "./http": {
      requestJson: async (path: string) => {
        paths.push(path);
        return pages.shift();
      },
    },
    "../../features/machines/machineContracts": contracts,
  });
  const api = harness.load(
    fileURLToPath(new URL("./machinesApi.ts", import.meta.url)),
  ) as unknown as typeof import("./machinesApi");
  return { ...api, paths };
}
test("hosted discovery reads every cursor before returning excluded node IDs", async () => {
  const api = setup([
    { machines: [{ node_id: "a" }], next_cursor: "next&cursor" },
    { machines: [{ node_id: "b" }], next_cursor: null },
  ]);
  assert.deepEqual(Array.from(await api.readHostedMachineIds()), ["a", "b"]);
  assert.ok(api.paths[1]!.includes("cursor=next%26cursor"));
});
test("partial or looping hosted discovery cannot expose management controls", async () => {
  for (const pages of [
    [{ machines: [], next_cursor: undefined }],
    [
      { machines: [], next_cursor: "loop" },
      { machines: [], next_cursor: "loop" },
    ],
  ]) {
    await assert.rejects(setup(pages).readHostedMachineIds());
  }
});

test("owner policy writes revalidate ownership and refuse concurrent policy changes", async () => {
  const api = setup([]);
  const original = contracts.machineNodeSchema.parse({
    id: "node",
    name: "Node",
    owner: { id: "me", kind: "user", display_name: "Me" },
    status: "active",
    is_connected: true,
  });
  api.machinesApi.authorizedNode = async () => ({
    node: { ...original, machine_confirm: "all" },
    inventory: { nodes: [], updates: [], userId: "me", features: [] },
  });
  await assert.rejects(
    api.machinesApi.policy(original, {
      machine_confirm: "changes",
      allow_single_user_saved_logins: false,
      acknowledge_single_user_risk: false,
    }),
    /changed elsewhere/,
  );
  assert.equal(api.paths.length, 0);
});
test("an account or route guard changing during authority read prevents dispatch", async () => {
  const api = setup([]);
  const original = contracts.machineNodeSchema.parse({
    id: "node",
    name: "Node",
    owner: { id: "me", kind: "user", display_name: "Me" },
    status: "active",
    is_connected: true,
  });
  let current = true;
  api.machinesApi.authorizedNode = async () => {
    current = false;
    return {
      node: original,
      inventory: { nodes: [], updates: [], userId: "me", features: [] },
    };
  };
  await assert.rejects(
    api.machinesApi.policy(
      original,
      {
        machine_confirm: "changes",
        allow_single_user_saved_logins: false,
        acknowledge_single_user_risk: false,
      },
      () => current,
    ),
    /context changed/,
  );
  assert.equal(api.paths.length, 0);
});
test("saved-login deletion refuses a changed resource and sends no DELETE", async () => {
  const api = setup([]);
  const login = contracts.savedLoginSchema.parse({
    id: "login",
    owner_id: "me",
    label: "Work",
    updated_at: "before",
    created_at: "now",
    allowed_origins: [],
    has_password: true,
    has_totp: false,
    confirm_each_sign_in: true,
  });
  api.machinesApi.logins = async () => [{ ...login, updated_at: "after" }];
  await assert.rejects(api.machinesApi.removeLogin(login), /changed/);
  assert.equal(api.paths.length, 0);
});

test("update confirmation cannot silently target a newly offered version", async () => {
  const api = setup([]);
  const original = contracts.machineNodeSchema.parse({
    id: "node",
    name: "Node",
    owner: { id: "me", kind: "user", display_name: "Me" },
    status: "active",
    is_connected: true,
  });
  const update = contracts.machineUpdateSchema.parse({
    node_id: "node",
    current_version: "1",
    target_version: "3",
    update_available: true,
    updater_ready: true,
    automatic: false,
    installation: "native",
    phase: "idle",
    settings_path: "/assistant/machines?machine=node",
  });
  api.machinesApi.authorizedNode = async () => ({
    node: original,
    inventory: {
      nodes: [original],
      updates: [update],
      userId: "me",
      features: [],
    },
  });
  await assert.rejects(api.machinesApi.update(original, "2"), /target version/);
  assert.equal(api.paths.length, 0);
});
