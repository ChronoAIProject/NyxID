import { z } from "zod";
import { requestJson } from "./http";
import { getAuthEpoch } from "../auth/authEpoch";

import {
  machineNodeSchema,
  machineOrgSchema,
  machineUpdateSchema,
  manageableMachines,
  MachineReviewRequired,
  machinePolicyInput,
  policySignature,
  savedLoginSchema,
  machineActivitySchema,
  machineUpdateResultSchema,
  updateInProgress,
  type MachineInventory,
  type MachineNode,
  type MachinePolicy,
  type SavedLogin,
} from "../../features/machines/machineContracts";

function writeGuard(current?: () => boolean) {
  const epoch = getAuthEpoch();
  return () => {
    if (getAuthEpoch() !== epoch || (current && !current()))
      throw new MachineReviewRequired(
        "Machine action context changed. Refresh before continuing.",
      );
  };
}

const hostedPage = z.object({
  machines: z.array(z.object({ node_id: z.string() })),
  next_cursor: z.string().nullable(),
});

export async function readHostedMachineIds(signal?: AbortSignal) {
  const ids = new Set<string>();
  const cursors = new Set<string>();
  let cursor: string | undefined;
  for (let page = 0; page < 100; page++) {
    const query = new URLSearchParams({ limit: "100" });
    if (cursor) query.set("cursor", cursor);
    const result = hostedPage.parse(
      await requestJson(`/remote-machines?${query}`, { signal }),
    );
    result.machines.forEach((row) => ids.add(row.node_id));
    if (result.next_cursor === null) return ids;
    if (!result.next_cursor || cursors.has(result.next_cursor))
      throw new MachineReviewRequired(
        "Hosted machine discovery is incomplete. Open Machines on the web.",
      );
    cursors.add(result.next_cursor);
    cursor = result.next_cursor;
  }
  throw new MachineReviewRequired(
    "Hosted machine discovery exceeded its page limit. Open Machines on the web.",
  );
}

export const machinesApi = {
  async authorizedNode(id: string, owner: string, signal?: AbortSignal) {
    const inventory = await machinesApi.inventory(signal);
    const node = inventory.nodes.find(
      (row) => row.id === id && row.owner.id === owner,
    );
    if (!node)
      throw new MachineReviewRequired(
        "Machine access changed. Refresh Machines before continuing.",
      );
    return { node, inventory };
  },
  async policy(
    original: MachineNode,
    input: MachinePolicy,
    current?: () => boolean,
  ) {
    const guard = writeGuard(current);
    guard();
    const { node } = await machinesApi.authorizedNode(
      original.id,
      original.owner.id,
    );
    if (policySignature(node) !== policySignature(original))
      throw new MachineReviewRequired(
        "Owner policy changed elsewhere. Reload before saving.",
      );
    guard();
    await requestJson(
      `/nodes/${encodeURIComponent(node.id)}/machine-settings`,
      { method: "PUT", body: machinePolicyInput(node, input) },
    );
  },
  async automatic(
    original: MachineNode,
    expected: boolean,
    automatic: boolean,
    current?: () => boolean,
  ) {
    const guard = writeGuard(current);
    guard();
    const { node, inventory } = await machinesApi.authorizedNode(
      original.id,
      original.owner.id,
    );
    const update = inventory.updates.find((row) => row.node_id === node.id);
    if (!update || update.automatic !== expected)
      throw new MachineReviewRequired(
        "Update policy changed or is unavailable. Reload before saving.",
      );
    guard();
    await requestJson(
      `/machines/${encodeURIComponent(node.id)}/update-policy`,
      { method: "PUT", body: { automatic } },
    );
  },
  async update(
    original: MachineNode,
    targetVersion: string,
    current?: () => boolean,
  ) {
    const guard = writeGuard(current);
    guard();
    const { node, inventory } = await machinesApi.authorizedNode(
      original.id,
      original.owner.id,
    );
    const update = inventory.updates.find((row) => row.node_id === node.id);
    if (
      !node.is_connected ||
      !update?.update_available ||
      updateInProgress(update) ||
      update.target_version !== targetVersion
    )
      throw new MachineReviewRequired(
        "This machine's update status changed. Refresh and review the target version.",
      );
    guard();
    return machineUpdateResultSchema.parse(
      await requestJson(`/machines/${encodeURIComponent(node.id)}/update`, {
        method: "POST",
        body: {},
      }),
    );
  },
  async logins(signal?: AbortSignal) {
    return savedLoginSchema
      .array()
      .parse(await requestJson("/saved-logins?available=true", { signal }));
  },
  async removeLogin(login: SavedLogin, current?: () => boolean) {
    const guard = writeGuard(current);
    guard();
    const latest = (await machinesApi.logins()).find(
      (row) => row.id === login.id,
    );
    if (
      !latest ||
      latest.owner_id !== login.owner_id ||
      latest.updated_at !== login.updated_at ||
      latest.label !== login.label
    )
      throw new MachineReviewRequired(
        "This saved login changed or is no longer available. Refresh and review it again.",
      );
    guard();
    await requestJson(`/saved-logins/${encodeURIComponent(login.id)}`, {
      method: "DELETE",
    });
  },
  async activity(
    id: string,
    owner: string,
    filter: string,
    before?: string,
    signal?: AbortSignal,
  ) {
    await machinesApi.authorizedNode(id, owner, signal);
    const query = new URLSearchParams({ limit: "50" });
    if (filter === "unattributed") query.set("unattributed", "true");
    else if (filter) query.set("agent_id", filter);
    if (before) query.set("before", before);
    return machineActivitySchema.parse(
      await requestJson(
        `/machines/${encodeURIComponent(id)}/activity?${query}`,
        { signal },
      ),
    );
  },
  async inventory(signal?: AbortSignal): Promise<MachineInventory> {
    const [rawNodes, rawOrgs, rawUser, hostedIds, rawUpdates] =
      await Promise.all([
        requestJson("/nodes", { signal }),
        requestJson("/orgs", { signal }),
        requestJson("/users/me", { signal }),
        readHostedMachineIds(signal),
        requestJson("/machines/updates", { signal }),
      ]);
    const nodes = z
      .object({ nodes: machineNodeSchema.array() })
      .parse(rawNodes).nodes;
    const orgs = z
      .object({ orgs: machineOrgSchema.array() })
      .parse(rawOrgs).orgs;
    const user = z
      .object({
        id: z.string(),
        capabilities: z
          .object({ enabled_features: z.string().array() })
          .optional(),
      })
      .parse(rawUser);
    return {
      nodes: manageableMachines(nodes, orgs, hostedIds, user.id),
      updates: machineUpdateSchema.array().parse(rawUpdates),
      userId: user.id,
      features: user.capabilities?.enabled_features ?? [],
    };
  },
};
