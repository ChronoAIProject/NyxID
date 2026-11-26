import { queryOptions } from "@tanstack/react-query";
import { authoredSkillPreviewSchema } from "@/schemas/agent-skills";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";
import { assistantJson } from "./assistant-http";

/** The dock and the full review card observe the same query and verdict. */
export function authoredSkillPreviewQuery(owner: string | undefined, card: NyxAgentAcknowledgement) {
  const reference = card.authored_skill!;
  return queryOptions({
    queryKey: ["authored-skill-review", owner, reference.proposal_id, card.id],
    queryFn: async () => authoredSkillPreviewSchema.parse(await assistantJson(
      `/assistant/nyxagent/agents/${encodeURIComponent(reference.agent_id)}/learning/proposals/${encodeURIComponent(reference.proposal_id)}?acknowledgement_id=${encodeURIComponent(card.id)}`,
    )),
    enabled: Boolean(owner),
    refetchInterval: (query) => query.state.data?.status === "publishing" && query.state.data?.lease_live
      ? Math.min(3000 * 2 ** Math.min(query.state.dataUpdateCount, 3), 24000) : false,
    retry: false,
    throwOnError: false,
  });
}
