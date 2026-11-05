import type { AssistantAgent } from "@/schemas/assistant-nyxagent";

/**
 * Who can widen a specialist's access, and why the agent itself cannot. Org
 * agents are maintained by the organization: only its maintainers approve.
 */
export function accessAuthorityCopy(
  agent: AssistantAgent,
  requestMode: "denied" | "routed_to_nyxbot" | undefined,
): string {
  const sandbox =
    "This specialist is sandboxed: it cannot grant access to itself or to other agents.";
  const direct =
    "When you talk to it directly, it tells you what it is missing and continues; you grant access below.";
  if (agent.owner_kind === "org") {
    const org = agent.owner_name ?? "the organization";
    return agent.can_maintain === false
      ? `${sandbox} Only a maintainer of ${org} (an Admin or Member) can approve or grant its access; you can view it but not change it.`
      : `${sandbox} Maintainers of ${org}, including you, approve and grant its access here; your personal NyxBot cannot approve it on the organization's behalf.`;
  }
  return requestMode === "routed_to_nyxbot"
    ? `${sandbox} ${direct} Only for work NyxBot assigned, or in a group with NyxBot, do its requests go to NyxBot, which approves or denies them.`
    : `${sandbox} ${direct} Its requests are never routed: only work NyxBot assigned, or work in a group NyxBot is in, reports the missing access to NyxBot instead.`;
}
