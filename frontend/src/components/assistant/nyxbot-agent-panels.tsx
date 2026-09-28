import { useState } from "react";
import { ChevronRight, PanelRightOpen, Users } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import {
  AGENT_KIND_LABEL,
  AGENT_STATUS_LABEL,
  agentTitle,
  channelPlatformName,
} from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";
import type {
  AssistantAgent,
  AssistantAgentKind,
  AssistantAgentStatus,
} from "@/schemas/assistant-nyxagent";

/** Running = working now; idle = alive, waiting; destroyed = read-only. */
export function AgentStatusDot({
  status,
  className,
}: {
  readonly status: AssistantAgentStatus;
  readonly className?: string;
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block h-1.5 w-1.5 shrink-0 rounded-full",
        status === "running" && "animate-pulse bg-success",
        status === "idle" && "bg-muted-foreground",
        status === "destroyed" && "border border-text-tertiary bg-transparent",
        className,
      )}
    />
  );
}

/** NyxBot is the identity (accent); specialists are neutral. */
export function AgentKindBadge({ kind }: { readonly kind: AssistantAgentKind }) {
  return (
    <Badge variant={kind === "nyxbot" ? "accent" : "secondary"}>{AGENT_KIND_LABEL[kind]}</Badge>
  );
}

/** A thread that came from a chat app (Telegram, ...). */
export function ChannelBadge({ platform }: { readonly platform: string }) {
  return <Badge variant="secondary">via {channelPlatformName(platform)}</Badge>;
}

/**
 * Which agent this thread talks to. Threads of a destroyed specialist are
 * read-only; the banner says so and the page disables the composer.
 */
export function ThreadHeader({
  name,
  handle,
  kind,
  agentId,
  destroyed,
  channelPlatform,
  onOpenDetails,
}: {
  /** The agent's display name (or handle). */
  readonly name: string;
  /** "@handle", shown when a display name hides it. */
  readonly handle?: string;
  readonly kind: AssistantAgentKind;
  /** Picks the specialist's avatar tint. */
  readonly agentId?: string;
  readonly destroyed: boolean;
  /** Set when the thread answers one of the user's channel bots. */
  readonly channelPlatform?: string | null;
  readonly onOpenDetails?: () => void;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <AgentAvatar agent={{ id: agentId ?? name, name, kind }} size="md" />
        <h2 className="min-w-0 truncate text-[13px] font-semibold text-foreground">{name}</h2>
        {handle ? <span className="shrink-0 text-[11px] text-text-tertiary">{handle}</span> : null}
        <AgentKindBadge kind={kind} />
        {channelPlatform ? <ChannelBadge platform={channelPlatform} /> : null}
        {onOpenDetails ? (
          <Button
            size="sm"
            variant="ghost"
            className="ml-auto"
            aria-label="Agent details"
            onClick={onOpenDetails}
          >
            <PanelRightOpen aria-hidden="true" />
            Details
          </Button>
        ) : null}
      </div>
      {destroyed ? (
        <p
          role="status"
          className="rounded-lg bg-overlay px-3 py-2 text-[11px] text-muted-foreground"
        >
          {name} was destroyed. Its access is revoked and this thread is read-only.
        </p>
      ) : null}
    </div>
  );
}

function excerpt(text: string, length = 140): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > length ? `${flat.slice(0, length - 1)}…` : flat;
}

/**
 * NyxBot's view of its specialists: what each is doing, its latest reply,
 * and permission requests waiting for a decision.
 */
export function TeamStrip({
  agents,
  onOpenConversation,
}: {
  readonly agents: readonly AssistantAgent[];
  readonly onOpenConversation: (conversationId: string) => void;
}) {
  const [open, setOpen] = useState(true);
  // Live specialists, most recently active first.
  const specialists = agents
    .filter((agent) => agent.kind === "specialist" && agent.status !== "destroyed")
    .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at));
  if (!specialists.length) return null;
  const requests = specialists.reduce((total, agent) => total + agent.pending_requests.length, 0);
  const running = specialists.filter((agent) => agent.status === "running").length;
  return (
    <section aria-label="Team" className="rounded-xl border border-border/50 bg-card">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 px-4 py-2.5 text-left text-[12px] text-muted-foreground transition-colors hover:text-foreground"
      >
        <ChevronRight className={cn("h-3 w-3 transition-transform", open && "rotate-90")} />
        <Users aria-hidden="true" className="h-3.5 w-3.5 text-text-tertiary" />
        <span className="font-medium text-foreground">Team</span>
        <span>
          {specialists.length} {specialists.length === 1 ? "specialist" : "specialists"}
          {running ? ` · ${String(running)} working` : ""}
        </span>
        {requests ? (
          <span className="ml-auto rounded-md border border-warning/30 bg-warning/10 px-1.5 text-[10px] font-medium text-warning">
            {requests} {requests === 1 ? "request" : "requests"}
          </span>
        ) : null}
      </button>
      {open ? (
        <ul className="assistant-scrollbar max-h-56 divide-y divide-border/30 overflow-y-auto border-t border-border/50">
          {specialists.map((agent) => (
            <li key={agent.id} className="space-y-1.5 px-4 py-2.5">
              <div className="flex items-center gap-2">
                <AgentStatusDot status={agent.status} />
                {agent.home_conversation_id ? (
                  <button
                    type="button"
                    onClick={() => onOpenConversation(agent.home_conversation_id!)}
                    className="min-w-0 truncate text-[12px] font-medium text-foreground hover:underline"
                  >
                    {agentTitle(agent)}
                  </button>
                ) : (
                  <span className="min-w-0 truncate text-[12px] font-medium text-foreground">
                    {agentTitle(agent)}
                  </span>
                )}
                <span className="text-[11px] text-text-tertiary">
                  {AGENT_STATUS_LABEL[agent.status]}
                </span>
              </div>
              {agent.last_reply ? (
                <p className="line-clamp-1 pl-3.5 text-[11px] text-muted-foreground">
                  {excerpt(agent.last_reply.text)}
                </p>
              ) : null}
              {agent.pending_requests.map((request) => (
                <div
                  key={request.request_id}
                  className="ml-3.5 flex items-center gap-2 rounded-lg border border-warning/15 bg-warning/[0.04] px-2.5 py-1.5"
                >
                  <span className="min-w-0 flex-1 truncate text-[11px] text-warning">
                    {request.summary}
                  </span>
                  <Button
                    size="sm"
                    variant="outline"
                    aria-label={`Review ${agentTitle(agent)}'s request: ${request.summary}`}
                    onClick={() => onOpenConversation(request.conversation_id)}
                  >
                    Review
                  </Button>
                </div>
              ))}
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}

/** Wake-up items the agent will see at the start of this thread's next turn. */
export function PendingEventsNote({
  count,
  agentName,
}: {
  readonly count: number;
  readonly agentName: string;
}) {
  if (count <= 0) return null;
  return (
    <p role="status" className="px-1 text-[11px] text-text-tertiary">
      {count} {count === 1 ? "update" : "updates"} waiting for {agentName}&apos;s next turn
    </p>
  );
}
