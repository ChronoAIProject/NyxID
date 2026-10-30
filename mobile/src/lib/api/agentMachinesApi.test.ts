import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import * as contracts from "../../features/machines/agentMachineContracts";
import { MachineReviewRequired } from "../../features/machines/machineContracts";

const machine = () =>
  contracts.agentMachineSchema.parse({
    node_id: "node",
    name: "Work",
    revision: 3,
    protocol_v2: true,
    can_edit: true,
    legacy: false,
    mode: "separated",
    capabilities: {
      shell: true,
      files: false,
      browser: true,
      computer: false,
      developer_browser: false,
    },
    ceiling: {
      shell: true,
      files: true,
      browser: true,
      computer: true,
      developer_browser: true,
    },
    saved_login_ids: ["login"],
    separated: { available: true, landlock_abi: 6, reason: null },
  });
function setup() {
  const writes: { path: string; options: any }[] = [];
  const harness = hookHarness({
    "../auth/authEpoch": { getAuthEpoch: () => 1 },
    "./agentsApi": {},
    "../../features/machines/machineContracts": { MachineReviewRequired },
    "../../features/machines/agentMachineContracts": contracts,
    "./http": {
      requestJson: async (path: string, options: any) => {
        writes.push({ path, options });
        return path.endsWith("context-request")
          ? {
              status: "pending",
              acknowledgement_id: "ack",
              conversation_id: "exact-home",
              message: "Pending",
            }
          : [machine()];
      },
    },
  });
  const { agentMachinesApi: api } = harness.load(
    fileURLToPath(new URL("./agentMachinesApi.ts", import.meta.url)),
  ) as unknown as typeof import("./agentMachinesApi");
  const data = {
    agent: {
      id: "agent",
      owner_id: "owner",
      status: "idle",
      can_maintain: true,
    },
    machines: [machine()],
    features: ["assistant:machine-capabilities", "assistant:machine-contexts"],
  };
  api.read = async () => data as Awaited<ReturnType<typeof api.read>>;
  return { api, data, writes };
}
test("capability edit sends only revisioned machine assignment and preserves login scope", async () => {
  const x = setup();
  await x.api.save(
    "agent",
    "owner",
    machine(),
    { ...machine().capabilities, shell: false },
    () => true,
  );
  assert.equal(
    x.writes[0]?.path,
    "/assistant/nyxagent/agents/agent/machines/node",
  );
  assert.deepEqual(Object.keys(x.writes[0]!.options.body).sort(), [
    "capabilities",
    "expected_revision",
    "saved_login_ids",
  ]);
  assert.deepEqual(x.writes[0]!.options.body.saved_login_ids, ["login"]);
});
test("stale revision, owner change, feature removal or lost context cannot dispatch", async () => {
  for (const change of ["revision", "owner", "feature", "context"]) {
    const x = setup();
    if (change === "revision") x.data.machines[0]!.revision++;
    if (change === "owner") x.data.agent.owner_id = "new-owner";
    if (change === "feature") x.data.features = [];
    await assert.rejects(
      x.api.save(
        "agent",
        "owner",
        machine(),
        machine().capabilities,
        () => change !== "context",
      ),
    );
    assert.equal(x.writes.length, 0);
  }
});
test("context mode request preserves assignment and returns the exact owner-card conversation", async () => {
  const x = setup();
  const result = await x.api.requestContext(
    "agent",
    "owner",
    machine(),
    "separated",
    () => true,
  );
  assert.equal(result.conversation_id, "exact-home");
  assert.equal(x.writes[0]!.options.method, "POST");
  assert.match(x.writes[0]!.path, /\/context-request$/);
  assert.deepEqual(
    x.writes[0]!.options.body.selection.capabilities,
    machine().capabilities,
  );
  assert.equal(x.writes[0]!.options.body.selection.mode, "separated");
});
