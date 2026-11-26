import { useQuery } from "@tanstack/react-query";
import type { AgentIdentity } from "../../components/agentAvatarModel";
import { agentsApi } from "../api/agentsApi";
import { NYXBOT_PLACEHOLDER, ownNyxBot } from "./ownNyxBot";

/**
 * The person's NyxBot as an avatar identity, from the roster the workspace
 * already caches. Never fetches: screens that show the roster load it.
 */
export function useOwnNyxBotIdentity(): AgentIdentity {
  const roster = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: false,
  });
  return ownNyxBot(roster.data?.agents) ?? NYXBOT_PLACEHOLDER;
}
