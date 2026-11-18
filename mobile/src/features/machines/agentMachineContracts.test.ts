import assert from "node:assert/strict";
import test from "node:test";
import {
  agentMachineSchema,
  capabilitySelection,
  machineAccessState,
  setMachineCapability,
} from "./agentMachineContracts";

export const agentMachineFixture = () =>
  agentMachineSchema.parse({
    node_id: "node",
    name: "Workstation",
    revision: 7,
    protocol_v2: true,
    can_edit: true,
    capabilities: {
      shell: true,
      files: true,
      browser: true,
      computer: true,
      developer_browser: true,
    },
    ceiling: {
      shell: true,
      files: true,
      browser: true,
      computer: true,
      developer_browser: true,
    },
    legacy: false,
    mode: "separated",
    saved_login_ids: ["login"],
    separated: { available: true, landlock_abi: 6, reason: null },
  });
test("browser dependency clears both desktop capabilities without widening others", () => {
  const machine = agentMachineFixture();
  const next = setMachineCapability(machine.capabilities, "browser", false);
  assert.deepEqual(next, {
    shell: true,
    files: true,
    browser: false,
    computer: false,
    developer_browser: false,
  });
  assert.equal(machine.capabilities.browser, true);
  assert.throws(
    () => capabilitySelection(machine, { ...next, computer: true }),
    /Secure browser/,
  );
});
test("selection binds revision, preserves saved logins, and obeys editing and node ceiling gates", () => {
  const machine = agentMachineFixture();
  const selection = capabilitySelection(machine, {
    ...machine.capabilities,
    shell: false,
  });
  assert.equal(selection.expected_revision, 7);
  assert.deepEqual(selection.saved_login_ids, ["login"]);
  assert.equal("services" in selection, false);
  for (const change of [
    { protocol_v2: false },
    { can_edit: false },
    { mode: "selection_required" as const },
  ]) {
    assert.throws(() =>
      capabilitySelection({ ...machine, ...change }, machine.capabilities),
    );
  }
  assert.throws(
    () =>
      capabilitySelection(
        { ...machine, ceiling: { ...machine.ceiling, shell: false } },
        machine.capabilities,
      ),
    /off on the machine/,
  );
});
test("selection, separation, cleanup and delivery remain distinguishable", () => {
  const machine = agentMachineFixture();
  assert.match(
    machineAccessState({ ...machine, selection_required: true }).join(" "),
    /Execution is blocked/,
  );
  assert.match(
    machineAccessState({
      ...machine,
      mode: "shared_legacy",
      legacy: true,
    }).join(" "),
    /Shared legacy/,
  );
  assert.match(
    machineAccessState({
      ...machine,
      separated: { available: false, landlock_abi: null, reason: "missing" },
    }).join(" "),
    /no automatic shared fallback/,
  );
  const pending = machineAccessState({
    ...machine,
    quarantine_pending: true,
    execution_revocation_pending: true,
    delivery_pending: true,
  }).join(" ");
  assert.match(pending, /Browser cleanup/);
  assert.match(pending, /commands are stopping/);
  assert.doesNotMatch(pending, /commands have stopped/);
  assert.match(
    machineAccessState({ ...machine, delivery_pending: true }).join(" "),
    /acknowledgement/,
  );
});
