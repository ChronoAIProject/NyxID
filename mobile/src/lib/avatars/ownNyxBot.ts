import type { AgentIdentity } from "../../components/agentAvatarModel";

/** NyxBot before the roster has loaded: its derived default look. */
export const NYXBOT_PLACEHOLDER: AgentIdentity = {
  id: "nyxbot",
  name: "NyxBot",
  kind: "nyxbot",
};

type RosterAgent = AgentIdentity & {
  readonly share_role?: string | null;
  readonly owner_kind?: string;
};

/** The signed-in person's own NyxBot in a roster (not a shared or org one). */
export function ownNyxBot<T extends RosterAgent>(
  agents: readonly T[] | undefined,
): T | undefined {
  return agents?.find(
    (agent) =>
      agent.kind === "nyxbot" && !agent.share_role && agent.owner_kind !== "org",
  );
}
