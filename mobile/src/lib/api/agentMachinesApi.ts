import { z } from "zod";
import { getAuthEpoch } from "../auth/authEpoch";
import { requestJson } from "./http";
import { agentsApi } from "./agentsApi";
import { MachineReviewRequired } from "../../features/machines/machineContracts";
import {
  agentMachineSchema,
  capabilitySelection,
  contextModeSchema,
  contextRequestSchema,
  separationAvailable,
  type AgentMachine,
  type ContextMode,
  type MachineCapabilities,
} from "../../features/machines/agentMachineContracts";

const path = (id: string) =>
  `/assistant/nyxagent/agents/${encodeURIComponent(id)}/machines`;
export const agentMachinesApi = {
  async read(agentId: string, signal?: AbortSignal) {
    const [detail, rawMachines, rawUser] = await Promise.all([
      agentsApi.detail(agentId, signal),
      requestJson(path(agentId), { signal }),
      requestJson("/users/me", { signal }),
    ]);
    if (detail.agent.status === "destroyed" || !detail.agent.can_maintain)
      throw new MachineReviewRequired(
        "This agent is destroyed or you no longer have maintainer access.",
      );
    const user = z
      .object({
        capabilities: z
          .object({ enabled_features: z.string().array() })
          .optional(),
      })
      .parse(rawUser);
    return {
      agent: detail.agent,
      machines: agentMachineSchema.array().parse(rawMachines),
      features: user.capabilities?.enabled_features ?? [],
    };
  },
  async save(
    agentId: string,
    ownerId: string | undefined,
    original: AgentMachine,
    caps: MachineCapabilities,
    current: () => boolean,
  ) {
    const epoch = getAuthEpoch();
    const data = await agentMachinesApi.read(agentId);
    const latest = data.machines.find(
      (row) => row.node_id === original.node_id,
    );
    if (!current() || epoch !== getAuthEpoch())
      throw new MachineReviewRequired(
        "Machine action context changed. Refresh before continuing.",
      );
    if (
      data.agent.owner_id !== ownerId ||
      !latest ||
      latest.revision !== original.revision
    )
      throw new MachineReviewRequired(
        "Machine access changed elsewhere. Reload and review before saving.",
      );
    if (!data.features.includes("assistant:machine-capabilities"))
      throw new MachineReviewRequired(
        "Machine capability editing is not enabled for this account.",
      );
    const body = capabilitySelection(latest, caps);
    // The revision is enforced by the server. This endpoint changes no service grants.
    return agentMachineSchema
      .array()
      .parse(
        await requestJson(
          `${path(agentId)}/${encodeURIComponent(original.node_id)}`,
          { method: "PUT", body },
        ),
      );
  },
  async requestContext(
    agentId: string,
    ownerId: string | undefined,
    original: AgentMachine,
    mode: ContextMode,
    current: () => boolean,
  ) {
    contextModeSchema.parse(mode);
    const epoch = getAuthEpoch();
    const data = await agentMachinesApi.read(agentId);
    const latest = data.machines.find(
      (row) => row.node_id === original.node_id,
    );
    if (!current() || epoch !== getAuthEpoch())
      throw new MachineReviewRequired(
        "Machine action context changed. Refresh before continuing.",
      );
    if (
      data.agent.owner_id !== ownerId ||
      !latest?.can_edit ||
      latest.revision !== original.revision
    )
      throw new MachineReviewRequired(
        "Machine access changed elsewhere. Reload and review before requesting owner approval.",
      );
    if (
      mode === "separated" &&
      (!data.features.includes("assistant:machine-contexts") ||
        !separationAvailable(latest))
    )
      throw new MachineReviewRequired(
        "Separated contexts are unavailable. No shared fallback is selected.",
      );
    return contextRequestSchema.parse(
      await requestJson(
        `${path(agentId)}/${encodeURIComponent(original.node_id)}/context-request`,
        {
          method: "POST",
          body: {
            selection: {
              expected_revision: latest.revision,
              capabilities: latest.capabilities,
              saved_login_ids: latest.saved_login_ids ?? [],
              mode,
            },
          },
        },
      ),
    );
  },
};
