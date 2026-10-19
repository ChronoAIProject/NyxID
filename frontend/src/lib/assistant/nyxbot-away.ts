import type { AssistantAgent, AssistantAgentRequest } from "@/schemas/assistant-nyxagent";

const AWAY_ITEMS = 6;

function excerpt(text: string, length = 160): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > length ? `${flat.slice(0, length - 1)}…` : flat;
}

export type AwayItem =
  | {
      readonly key: string;
      readonly kind: "request";
      readonly agent: AssistantAgent;
      readonly request: AssistantAgentRequest;
    }
  | {
      readonly key: string;
      readonly kind: "update";
      readonly agent: AssistantAgent;
      readonly text: string;
      readonly at: string;
    };

/**
 * What happened while the user was elsewhere: requests that need a decision
 * first, then each specialist's latest reply or current work, newest first.
 */
export function awayItems(agents: readonly AssistantAgent[]): AwayItem[] {
  const specialists = agents.filter(
    (agent) => agent.kind === "specialist" && agent.status !== "destroyed",
  );
  const requests: AwayItem[] = specialists.flatMap((agent) =>
    agent.pending_requests.map((request) => ({
      key: `request:${request.request_id}`,
      kind: "request" as const,
      agent,
      request,
    })),
  );
  const updates: AwayItem[] = specialists
    .filter((agent) => agent.last_reply || agent.status === "running")
    .map((agent) => ({
      key: `update:${agent.id}`,
      kind: "update" as const,
      agent,
      text: agent.last_reply ? excerpt(agent.last_reply.text) : "Working on a task.",
      at:
        agent.status === "running" || !agent.last_reply
          ? agent.last_active_at
          : agent.last_reply.created_at,
    }))
    .sort((a, b) => b.at.localeCompare(a.at));
  return [...requests, ...updates].slice(0, AWAY_ITEMS);
}
