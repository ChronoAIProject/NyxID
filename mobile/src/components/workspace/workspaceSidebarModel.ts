import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import type { AssistantConversation } from "../../features/assistant/assistantTypes";
import type { AssistantAgent } from "../../features/agents/agentContracts";
import type { Group } from "../../features/groups/groupContracts";
import { threadStatus, threadTitle } from "./workspaceChatListModel";

export const SIDEBAR_BAR_GAP = 12;
export function sidebarBottomSpace(barHeight: number, safeBottom: number) {
  return Math.max(0, barHeight) + Math.max(0, safeBottom) + SIDEBAR_BAR_GAP * 2;
}
export type WorkspaceRecent = {
  key: string;
  title: string;
  activityAt: string;
  status?: string;
  target: WorkspaceChatTarget;
};
export function workspaceRecents(
  conversations: readonly AssistantConversation[],
  crews: readonly Group[],
): WorkspaceRecent[] {
  const rows = new Map<string, WorkspaceRecent>();
  for (const chat of conversations) {
    if (!chat.message_count) continue;
    const key = `chat:${chat.id}`;
    const previous = rows.get(key);
    if (previous && previous.activityAt >= chat.last_message_at) continue;
    rows.set(key, {
      key,
      title: threadTitle(chat),
      activityAt: chat.last_message_at,
      status: threadStatus(chat),
      target: {
        kind: "agent",
        agentId: chat.agent?.id,
        conversationId: chat.id,
      },
    });
  }
  for (const crew of crews) {
    if (!crew.last_message_at || !crew.message_count) continue;
    const key = `crew:${crew.id}`;
    rows.set(key, {
      key,
      title: crew.name,
      activityAt: crew.last_message_at,
      status: crew.working_agent_ids.length ? "Running" : undefined,
      target: { kind: "group", groupId: crew.id },
    });
  }
  return [...rows.values()].sort(
    (a, b) =>
      b.activityAt.localeCompare(a.activityAt) || a.key.localeCompare(b.key),
  );
}
export function sidebarAgentOwner(agent: AssistantAgent) {
  return agent.owner_kind === "org"
    ? {
        key: `org:${agent.owner_id || agent.owner_name || agent.id}`,
        label: agent.owner_name || "Organization",
      }
    : { key: "personal", label: "Personal" };
}
export function sidebarOrderedAgents(agents: readonly AssistantAgent[]) {
  return [...agents].sort((a, b) => {
    const first = sidebarAgentOwner(a),
      second = sidebarAgentOwner(b);
    if (first.key !== second.key) {
      if (first.key === "personal") return -1;
      if (second.key === "personal") return 1;
      return (
        first.label.localeCompare(second.label) ||
        first.key.localeCompare(second.key)
      );
    }
    if (a.kind !== b.kind) return a.kind === "nyxbot" ? -1 : 1;
    return (a.display_name || a.name).localeCompare(b.display_name || b.name);
  });
}
