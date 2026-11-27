import { createContext, useContext } from "react";

export const AGENT_AVATARS_FLAG = "assistant:avatars";

const AgentAvatarsContext = createContext(false);

/** Whether `assistant:avatars` is on for the signed-in person; supplied by
 * `AgentAvatarsProvider` (off without it). */
export function useAgentAvatarsEnabled(): boolean {
  return useContext(AgentAvatarsContext);
}

/** Supplies the flag value (the provider, tests and previews). */
export const AgentAvatarsFlag = AgentAvatarsContext.Provider;
