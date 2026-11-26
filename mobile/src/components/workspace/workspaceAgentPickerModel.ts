import type { AssistantAgent } from "../../features/agents/agentContracts";
import type { WorkspaceRecent } from "./workspaceSidebarModel";

/** Recents already owns activity ordering. Resolve against today's usable roster,
 * never the historical agent snapshot. Legacy unscoped threads belong to NyxBot. */
export function workspaceAgentPickerSections(
  agents: readonly AssistantAgent[],
  rows: readonly WorkspaceRecent[],
) {
  const usable = agents.filter((agent) => agent.can_use === true && agent.status !== "destroyed");
  const byId = new Map(usable.map((agent) => [agent.id, agent]));
  const nyxbot = usable.find((agent) => agent.kind === "nyxbot");
  const seen = new Set<string>();
  const recent: { agent: AssistantAgent; row: WorkspaceRecent }[] = [];
  for (const row of rows) {
    if (row.target.kind !== "agent" || !row.target.conversationId) continue;
    const agent = row.target.agentId ? byId.get(row.target.agentId) : nyxbot;
    if (!agent || seen.has(agent.id)) continue;
    seen.add(agent.id);
    recent.push({ agent, row: { ...row, target: { ...row.target, agentId: agent.id } } });
    if (recent.length === 3) break;
  }
  return { recent, roster: agents.filter((agent) => !seen.has(agent.id)) };
}
