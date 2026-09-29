import { useState, type ReactNode } from "react";
import { MessageSquare, Plus } from "lucide-react";
import { AgentAvatar, AgentAvatarStack } from "@/components/assistant/nyxbot-agent-avatar";
import { AgentStatusDot } from "@/components/assistant/nyxbot-agent-panels";
import { Button } from "@/components/ui/button";
import { nyxBotOf, useNyxBotAgent } from "@/hooks/use-nyxbot-agents";
import { awayItems } from "@/lib/assistant/nyxbot-away";
import {
  AGENT_STATUS_LABEL,
  agentHandle,
  agentTitle,
  greetingFor,
} from "@/lib/assistant/nyxbot-labels";
import { formatRelativeTime } from "@/lib/utils";
import type { AssistantAgent, AssistantGroup } from "@/schemas/assistant-nyxagent";

const MEMORY_PREVIEW = 3;


function Section({
  id,
  title,
  action,
  children,
}: {
  readonly id: string;
  readonly title: string;
  readonly action?: ReactNode;
  readonly children: ReactNode;
}) {
  return (
    <section aria-labelledby={id} className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <h2 id={id} className="text-[15px] font-semibold text-foreground">
          {title}
        </h2>
        {action}
      </div>
      {children}
    </section>
  );
}

function Quiet({ children }: { readonly children: ReactNode }) {
  return (
    <p className="rounded-lg bg-overlay px-4 py-3 text-[12px] text-muted-foreground">{children}</p>
  );
}

function AwayList({
  agents,
  onOpenAgent,
  onOpenConversation,
}: {
  readonly agents: readonly AssistantAgent[];
  readonly onOpenAgent: (agent: AssistantAgent) => void;
  readonly onOpenConversation: (conversationId: string) => void;
}) {
  const items = awayItems(agents);
  if (!items.length) {
    return <Quiet>All caught up. Replies and requests from your specialists show up here.</Quiet>;
  }
  return (
    <ul className="divide-y divide-border/30 overflow-hidden rounded-xl border border-border/50 bg-card">
      {items.map((item) => (
        <li key={item.key} className="flex items-start gap-3 px-4 py-3">
          <AgentAvatar agent={item.agent} size="lg" />
          <div className="min-w-0 flex-1 space-y-0.5">
            <div className="flex items-center gap-2 text-[12px]">
              <span className="font-medium text-foreground">{agentTitle(item.agent)}</span>
              {item.kind === "request" ? (
                <span className="rounded-md border border-warning/30 bg-warning/10 px-1.5 text-[10px] font-medium text-warning light:border-transparent light:bg-warning light:text-white">
                  Needs a decision
                </span>
              ) : (
                <span className="flex items-center gap-1.5 text-[11px] text-text-tertiary">
                  <AgentStatusDot status={item.agent.status} />
                  {AGENT_STATUS_LABEL[item.agent.status]}
                  <span aria-hidden="true">·</span>
                  {formatRelativeTime(item.at)}
                </span>
              )}
            </div>
            <p className="line-clamp-2 text-[12px] text-muted-foreground">
              {item.kind === "request" ? item.request.summary : item.text}
            </p>
          </div>
          {item.kind === "request" ? (
            <Button
              size="sm"
              variant="outline"
              aria-label={`Review ${agentTitle(item.agent)}'s request: ${item.request.summary}`}
              onClick={() => onOpenConversation(item.request.conversation_id)}
            >
              Review
            </Button>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              aria-label={`Open ${agentTitle(item.agent)}`}
              onClick={() => onOpenAgent(item.agent)}
            >
              Open
            </Button>
          )}
        </li>
      ))}
    </ul>
  );
}

function MemoryPreview({
  nyxbot,
  onManage,
}: {
  readonly nyxbot: AssistantAgent | undefined;
  readonly onManage: () => void;
}) {
  const detail = useNyxBotAgent(nyxbot?.id);
  const notes = detail.data?.memory ?? [];
  return (
    <Section
      id="nyxbot-home-memory"
      title="What NyxBot remembers"
      action={
        <Button size="sm" variant="ghost" onClick={onManage} disabled={!nyxbot}>
          Manage
        </Button>
      }
    >
      {detail.isPending && nyxbot ? (
        <Quiet>Loading memory...</Quiet>
      ) : notes.length ? (
        <ul
          aria-label="NyxBot memory preview"
          className="divide-y divide-border/30 overflow-hidden rounded-xl border border-border/50 bg-card"
        >
          {notes.slice(0, MEMORY_PREVIEW).map((note) => (
            <li key={note.id} className="line-clamp-2 px-4 py-2.5 text-[12px] text-foreground">
              {note.text}
            </li>
          ))}
          {notes.length > MEMORY_PREVIEW ? (
            <li className="px-4 py-2 text-[11px] text-text-tertiary">
              and {notes.length - MEMORY_PREVIEW} more
            </li>
          ) : null}
        </ul>
      ) : (
        <Quiet>
          Nothing yet. Tell NyxBot &ldquo;Remember …&rdquo; and it keeps that across every chat.
        </Quiet>
      )}
    </Section>
  );
}

function RosterCard({
  agent,
  onChat,
}: {
  readonly agent: AssistantAgent;
  readonly onChat: (agent: AssistantAgent) => void;
}) {
  const nyxbot = agent.kind === "nyxbot";
  const handle = agentHandle(agent);
  return (
    <li className="flex flex-col gap-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="flex items-start gap-3">
        <AgentAvatar agent={agent} size="lg" />
        <div className="min-w-0 flex-1">
          <p className="flex min-w-0 items-baseline gap-1.5">
            <span className="truncate text-[13px] font-semibold text-foreground">
              {agentTitle(agent)}
            </span>
            {handle ? (
              <span className="shrink-0 text-[11px] text-text-tertiary">{handle}</span>
            ) : null}
          </p>
          <p className="flex items-center gap-1.5 text-[11px] text-text-tertiary">
            {nyxbot ? (
              "Your personal agent"
            ) : (
              <>
                <AgentStatusDot status={agent.status} />
                {AGENT_STATUS_LABEL[agent.status]}
                {agent.pending_requests.length ? (
                  <span className="text-warning">
                    · {agent.pending_requests.length}{" "}
                    {agent.pending_requests.length === 1 ? "request" : "requests"}
                  </span>
                ) : null}
              </>
            )}
          </p>
        </div>
      </div>
      <p className="line-clamp-2 min-h-8 text-[12px] text-muted-foreground">
        {agent.description ||
          (nyxbot
            ? "Full access to your services and account. Delegates to your specialists."
            : "No role described yet.")}
      </p>
      <div className="flex justify-end">
        <Button
          size="sm"
          variant="outline"
          aria-label={`Chat with ${agentTitle(agent)}`}
          onClick={() => onChat(agent)}
        >
          <MessageSquare aria-hidden="true" />
          Chat
        </Button>
      </div>
    </li>
  );
}

function GroupCard({
  group,
  onOpen,
}: {
  readonly group: AssistantGroup;
  readonly onOpen: () => void;
}) {
  const working = group.working_agent_ids.length;
  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        aria-label={`Open group ${group.name}`}
        className="flex w-full flex-col gap-2.5 rounded-xl border border-border/50 bg-card p-4 text-left transition-colors hover:bg-overlay"
      >
        <div className="flex w-full items-center gap-2">
          <AgentAvatarStack agents={group.members} size="sm" max={4} />
          {working ? (
            <span className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <span aria-hidden="true" className="h-1.5 w-1.5 animate-pulse rounded-full bg-success" />
              {working} working
            </span>
          ) : null}
        </div>
        <span className="truncate text-[13px] font-semibold text-foreground">{group.name}</span>
        <span className="truncate text-[11px] text-text-tertiary">
          {group.members.map((member) => agentTitle(member)).join(", ")}
          {group.last_message_at ? ` · ${formatRelativeTime(group.last_message_at)}` : ""}
        </span>
      </button>
    </li>
  );
}

/**
 * The NyxBot home: a greeting, what happened while the user was away, what
 * NyxBot remembers, the roster and the groups. The page renders the
 * "Message NyxBot" composer below it.
 */
export function NyxBotHome({
  userName,
  agents,
  agentsLoading,
  groups,
  bottomInset,
  onChat,
  onOpenConversation,
  onOpenGroup,
  onNewGroup,
  onNewAgent,
  onManageMemory,
}: {
  readonly userName: string | undefined;
  readonly agents: readonly AssistantAgent[];
  readonly agentsLoading: boolean;
  readonly groups: readonly AssistantGroup[];
  readonly bottomInset: number;
  readonly onChat: (agent: AssistantAgent) => void;
  readonly onOpenConversation: (conversationId: string) => void;
  readonly onOpenGroup: (groupId: string) => void;
  readonly onNewGroup: () => void;
  readonly onNewAgent: () => void;
  readonly onManageMemory: () => void;
}) {
  const [now] = useState(() => new Date());
  const firstName = userName?.trim().split(/\s+/)[0];
  const nyxbot = nyxBotOf(agents);
  const roster = agents
    .filter((agent) => agent.status !== "destroyed")
    .sort((a, b) => Number(b.kind === "nyxbot") - Number(a.kind === "nyxbot"));
  return (
    <div className="assistant-scrollbar min-h-0 flex-1 overflow-y-auto px-4 sm:px-6">
      {/* Same column as the composer below it: its 758px rail, side padding and 30px gutter. */}
      <div
        className="mx-auto flex w-full max-w-[758px] flex-col gap-8 pl-[30px] pt-8 sm:pl-[54px] sm:pr-6"
        style={{ paddingBottom: Math.max(bottomInset + 24, 96) }}
      >
        <header className="flex items-center gap-4">
          <AgentAvatar agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }} size="lg" />
          <div className="min-w-0 space-y-1">
            <h1
              className="text-[22px] font-bold leading-[1.1] text-foreground sm:text-[28px]"
              style={{ letterSpacing: "-0.03em" }}
            >
              {greetingFor(now)}
              {firstName ? `, ${firstName}` : ""}
            </h1>
            <p className="text-[12px] text-muted-foreground">
              NyxBot is your personal agent. Ask it anything below, or check in on your team.
            </p>
          </div>
        </header>

        <Section id="nyxbot-home-away" title="While you were away">
          {agentsLoading ? (
            <Quiet>Loading your agents...</Quiet>
          ) : (
            <AwayList
              agents={agents}
              onOpenAgent={onChat}
              onOpenConversation={onOpenConversation}
            />
          )}
        </Section>

        <MemoryPreview nyxbot={nyxbot} onManage={onManageMemory} />

        <Section
          id="nyxbot-home-agents"
          title="Your agents"
          action={
            <Button size="sm" variant="ghost" onClick={onNewAgent}>
              <Plus aria-hidden="true" />
              New agent
            </Button>
          }
        >
          {roster.length ? (
            <ul aria-label="Agents" className="grid gap-3 sm:grid-cols-2">
              {roster.map((agent) => (
                <RosterCard key={agent.id} agent={agent} onChat={onChat} />
              ))}
            </ul>
          ) : (
            <Quiet>{agentsLoading ? "Loading your agents..." : "No agents yet."}</Quiet>
          )}
        </Section>

        <Section
          id="nyxbot-home-groups"
          title="Groups"
          action={
            <Button size="sm" variant="ghost" onClick={onNewGroup}>
              <Plus aria-hidden="true" />
              New group
            </Button>
          }
        >
          {groups.length ? (
            <ul aria-label="Groups" className="grid gap-3 sm:grid-cols-2">
              {groups.map((group) => (
                <GroupCard key={group.id} group={group} onOpen={() => onOpenGroup(group.id)} />
              ))}
            </ul>
          ) : (
            <Quiet>
              Put several agents in one chat. Mention one with @ to ask it directly; NyxBot
              answers the rest.
            </Quiet>
          )}
        </Section>
      </div>
    </div>
  );
}
