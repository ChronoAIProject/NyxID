import { translateApp } from "../../i18n/appTranslation";
import type { AssistantAgent } from "../agents/agentContracts";

export const AWAY_PREVIEW = 6;

/** One complete phrase per time of day, with the first name when known. */
export function greetingFor(date: Date, name?: string) {
  const hour = date.getHours();
  const part = hour < 5 || hour >= 18 ? "evening" : hour < 12 ? "morning" : "afternoon";
  if (name) {
    if (part === "evening") return translateApp("Good evening, {{name}}", { name });
    if (part === "morning") return translateApp("Good morning, {{name}}", { name });
    return translateApp("Good afternoon, {{name}}", { name });
  }
  if (part === "evening") return translateApp("Good evening");
  return part === "morning" ? translateApp("Good morning") : translateApp("Good afternoon");
}

export function showNyxBotHome(scope: {
  home: boolean;
  contextual: boolean;
  nyxbot: boolean;
  conversationId?: string;
  routeConversationId?: string;
  openLatest?: boolean;
  sending: boolean;
  loading: boolean;
  inputText?: string;
  inputAttachments: readonly unknown[];
}) {
  return (
    scope.home &&
    !scope.contextual &&
    scope.nyxbot &&
    !scope.conversationId &&
    !scope.routeConversationId &&
    !scope.openLatest &&
    !scope.sending &&
    !scope.loading &&
    !scope.inputText &&
    scope.inputAttachments.length === 0
  );
}

export type AwayItem =
  | {
      key: string;
      kind: "request";
      agent: AssistantAgent;
      request: AssistantAgent["pending_requests"][number];
    }
  | {
      key: string;
      kind: "update";
      agent: AssistantAgent;
      text: string;
      at?: string;
    };

/** Match the web: decisions first, then each specialist's latest work. */
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
    .map((agent) => {
      const flat = agent.last_reply?.text.replace(/\s+/gu, " ").trim();
      return {
        key: `update:${agent.id}`,
        kind: "update" as const,
        agent,
        text:
          flat !== undefined
            ? flat.length > 160
              ? `${flat.slice(0, 159)}…`
              : flat
            : translateApp("Working on a task."),
        at:
          agent.status === "running"
            ? agent.last_active_at
            : agent.last_reply?.created_at,
      };
    })
    .sort((a, b) => (b.at ?? "").localeCompare(a.at ?? ""));
  return [...requests, ...updates];
}

/** Owner IDs keep organizations with matching names in separate sections. */
export function agentOwnerSections(agents: readonly AssistantAgent[]) {
  const sections = new Map<
    string,
    { id: string; label: string; agents: AssistantAgent[] }
  >();
  for (const agent of agents.filter((agent) => agent.status !== "destroyed")) {
    const id =
      agent.owner_kind === "org" ? (agent.owner_id ?? "org") : "personal";
    const section = sections.get(id) ?? {
      id,
      label: id === "personal" ? translateApp("Personal") : (agent.owner_name ?? id),
      agents: [],
    };
    section.agents.push(agent);
    sections.set(id, section);
  }
  for (const section of sections.values())
    section.agents.sort(
      (a, b) => Number(b.kind === "nyxbot") - Number(a.kind === "nyxbot"),
    );
  return [...sections.values()].sort(
    (a, b) => Number(b.id === "personal") - Number(a.id === "personal"),
  );
}
