import { z } from "zod";
import { requestJson } from "./http";
import {
  agentSkillsSchema,
  attachedSkillPreviewSchema,
  skillHistorySchema,
  skillPreviewSchema,
  skillSearchSchema,
  skillVersionsSchema,
  type AgentSkill,
} from "../../features/plugins/pluginSkillsTypes";
import { authoredSkillPreviewSchema } from "../../features/plugins/authoredSkillReviewState";

const ROOT = "/assistant/nyxagent";
const agentPath = (id: string) => `${ROOT}/agents/${encodeURIComponent(id)}`;
const skillsPath = (id: string) => `${agentPath(id)}/skills`;
const proposalPath = (id: string, proposalId: string) =>
  `${agentPath(id)}/learning/proposals/${encodeURIComponent(proposalId)}`;
export const pluginSkillsApi = {
  async authoredReview(
    agentId: string,
    proposalId: string,
    acknowledgementId: string,
    signal?: AbortSignal,
  ) {
    const result = authoredSkillPreviewSchema.parse(
      await requestJson(
        `${proposalPath(agentId, proposalId)}?acknowledgement_id=${encodeURIComponent(acknowledgementId)}`,
        { signal },
      ),
    );
    if (result.id !== proposalId || result.agent_id !== agentId)
      throw new Error(
        "The server returned a different skill draft. Reload the confirmation.",
      );
    return result;
  },
  async authoredAction(
    agentId: string,
    proposalId: string,
    action: "approve" | "reject" | "reprepare",
    body: Record<string, unknown>,
  ) {
    const result = await requestJson<unknown>(
      `${proposalPath(agentId, proposalId)}/${action}`,
      { method: "POST", body },
    );
    return z
      .object({ card_pending: z.boolean().optional() })
      .nullish()
      .parse(result);
  },
  async current(agentId: string, signal?: AbortSignal) {
    const result = agentSkillsSchema.parse(
      await requestJson(skillsPath(agentId), { signal }),
    );
    if (result.agent_id && result.agent_id !== agentId)
      throw new Error(
        "The server returned another agent's skills. Reload to continue.",
      );
    return result;
  },
  async write(
    agentId: string,
    body: { expected_revision: number; skills: AgentSkill[] },
  ) {
    await requestJson(skillsPath(agentId), { method: "PUT", body });
  },
  async search(query: string, page: number, signal?: AbortSignal) {
    return skillSearchSchema.parse(
      await requestJson(
        `${ROOT}/skills/catalog?q=${encodeURIComponent(query)}&page=${page}`,
        { signal },
      ),
    );
  },
  async versions(skillId: string, page: number, signal?: AbortSignal) {
    return skillVersionsSchema.parse(
      await requestJson(
        `${ROOT}/skills/catalog?skill=${encodeURIComponent(skillId)}&page=${page}`,
        { signal },
      ),
    );
  },
  async preview(skillId: string, version: string, signal?: AbortSignal) {
    return skillPreviewSchema.parse(
      await requestJson(
        `${ROOT}/skills/catalog?skill=${encodeURIComponent(skillId)}&version=${encodeURIComponent(version)}`,
        { signal },
      ),
    );
  },
  async test(agentId: string, skillId: string, signal?: AbortSignal) {
    return attachedSkillPreviewSchema.parse(
      await requestJson(
        `${skillsPath(agentId)}/${encodeURIComponent(skillId)}/preview`,
        { signal },
      ),
    );
  },
  async history(agentId: string, signal?: AbortSignal) {
    return skillHistorySchema.parse(
      await requestJson(`${skillsPath(agentId)}/history`, { signal }),
    );
  },
};
