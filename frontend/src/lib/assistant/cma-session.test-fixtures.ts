import type { CmaSessionReference } from "@/schemas/cma-session";

export const cmaSession: CmaSessionReference = {
  connection: {
    id: "22ec5c70-b8bb-4133-8824-031a1dc63945",
    name: "CMA",
    slug: "cma",
    connected: true,
    requires_connection: true,
  },
  agentId: "agt_observed",
  title: "Review the pull request",
  routes: {
    agents: "/api/v1/agents",
    agent: "/api/v1/agents/{id}",
    head: "/api/v2/agents/{agent_id}/events/head",
    events: "/api/v2/agents/{agent_id}/events",
  },
};

export function cmaEvent(
  sequence: number,
  kind: ({ type: string } & Record<string, unknown>) | null,
) {
  return {
    agent_id: cmaSession.agentId,
    incarnation: "incarnation-1",
    cursor: `cursor-${sequence}`,
    sequence,
    appended_at: "2026-11-03T04:05:06Z",
    kind,
  };
}

export function cmaFrame(
  sequence: number,
  kind: ({ type: string } & Record<string, unknown>) | null,
) {
  return `event: cma.agent.event\nid: cursor-${sequence}\ndata: ${JSON.stringify(cmaEvent(sequence, kind))}\n\n`;
}

export const cmaHead = {
  object: "cma.agent.event_head",
  schema_version: 1,
  agent_id: cmaSession.agentId,
  incarnation: "incarnation-1",
  through_cursor: "cursor-15",
  active_turn: cmaEvent(10, { type: "turn_started", turn_sequence: 10 }),
};
