import type { AssistantAgent } from "../agents/agentContracts";

/** A saved thread keeps its agent; a new draft uses the selected agent or NyxBot. */
export function resolveChatAgentId(
  agents: readonly Pick<AssistantAgent, "id" | "kind">[] | undefined,
  conversationAgentId: string | undefined,
  routeAgentId: string | undefined,
): string | undefined {
  return (
    conversationAgentId ??
    routeAgentId ??
    agents?.find((agent) => agent.kind === "nyxbot")?.id
  );
}

export function chatAgentLabel(
  agent: { name: string; display_name?: string | null } | null | undefined,
): string {
  return agent?.display_name?.trim() || agent?.name?.trim() || "NyxBot";
}
