import {
  agentRosterMood,
  ROSTER_ACTIVITY_LABELS,
} from "../../../../shared/agent-avatars/activity";
import type { AssistantAgent } from "./agentContracts";

export function rosterAvatarMood(agent: AssistantAgent) {
  return agentRosterMood({
    agentId: agent.id,
    activity: agent.activity,
    pendingOwnerActions: agent.pending_acknowledgements,
    running: agent.status === "running",
    destroyed: agent.status === "destroyed",
  });
}

export function rosterActivityLabel(mood: ReturnType<typeof rosterAvatarMood>) {
  return mood === "idle" ? "Idle" : ROSTER_ACTIVITY_LABELS[mood];
}
