import { useContext, type ReactNode } from "react";
import { ChatHeaderCompactContext } from "./chat-header-context";
import { PanelRightOpen } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import {
  AGENT_KIND_LABEL,
  channelPlatformName,
} from "@/lib/assistant/nyxbot-labels";
import { cn, formatTimeDistance } from "@/lib/utils";
import type {
  AssistantAgentKind,
  AssistantAgentStatus,
  NyxAgentWaiting,
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

export function ThreadPanels({ children }: { readonly children: ReactNode }) {
  const compact = useContext(ChatHeaderCompactContext);
  return (
    <div
      data-chat-thread-panels=""
      className={cn("shrink-0 px-4 sm:px-6", !compact && "pt-3")}
      style={{ width: "calc(100% - var(--assistant-content-inset, 0px))" }}
    >
      <div className="mx-auto w-full max-w-[var(--chat-content-width,758px)] space-y-2">{children}</div>
    </div>
  );
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
  controls,
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
  /** Additional thread controls. */
  readonly controls?: ReactNode;
}) {
  const compact = useContext(ChatHeaderCompactContext);
  return (
    <div className="space-y-2">
      <div
        role="group"
        aria-label="Conversation controls"
        data-compact={compact}
        className={cn(
          "flex items-center gap-2",
          compact
            ? "absolute top-2 right-[calc(var(--spacing)*4+var(--assistant-content-inset,0px))] z-20 max-w-[calc(100%-2rem)] rounded-xl border border-hairline bg-background/95 px-2 py-1 shadow-sm backdrop-blur-sm sm:right-[calc(var(--spacing)*6+var(--assistant-content-inset,0px))]"
            : "flex-wrap",
        )}
      >
        <AgentAvatar agent={{ id: agentId ?? name, name, kind }} size={compact ? "sm" : "md"} />
        <h2 className={cn("min-w-0 truncate font-semibold text-foreground", compact ? "max-w-24 text-11" : "text-13")}>{name}</h2>
        {handle && !compact ? <span className="shrink-0 text-11 text-text-tertiary">{handle}</span> : null}
        {!compact ? <AgentKindBadge kind={kind} /> : null}
        {channelPlatform && !compact ? <ChannelBadge platform={channelPlatform} /> : null}
        {controls || onOpenDetails ? (
          <div className="ml-auto flex flex-wrap items-center gap-1.5">
            {controls}
            {onOpenDetails ? (
              <Button
                size="sm"
                variant="ghost"
                aria-label="Agent details"
                title="Agent details"
                className={compact ? "h-7 w-7 px-0" : undefined}
                onClick={onOpenDetails}
              >
                <PanelRightOpen aria-hidden="true" />
                <span className={compact ? "sr-only" : undefined}>Details</span>
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
      {destroyed ? (
        <p
          role="status"
          className="rounded-lg bg-overlay px-3 py-2 text-11 text-muted-foreground"
        >
          {name} was destroyed. Its access is revoked and this thread is read-only.
        </p>
      ) : null}
    </div>
  );
}

/**
 * What the thread is waiting for outside the chat (a bot being created, a
 * service being connected, the owner verifying a chat app). The agent
 * continues by itself when it happens, so the user never has to say "done".
 */
export function WaitingNote({
  items,
  agentName,
}: {
  readonly items: readonly NyxAgentWaiting[];
  readonly agentName: string;
}) {
  if (!items.length) return null;
  return (
    <div
      role="status"
      aria-label="Waiting"
      className="space-y-1 rounded-lg border border-border px-3 py-2"
    >
      {items.map((item) => (
        <div key={`${item.kind}:${item.since}:${item.title}`}>
          <p className="flex items-center gap-2 text-12 text-foreground">
            <span
              aria-hidden="true"
              className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-success"
            />
            <span className="min-w-0 truncate">{item.title}</span>
            {item.expires_at ? (
              <span className="ml-auto shrink-0 text-11 text-text-tertiary">
                expires {formatTimeDistance(item.expires_at)}
              </span>
            ) : null}
          </p>
          {item.detail ? (
            <p className="pl-3.5 text-11 text-muted-foreground">{item.detail}</p>
          ) : null}
        </div>
      ))}
      <p className="text-11 text-text-tertiary">
        {agentName} continues here by itself when this happens; no need to reply.
      </p>
    </div>
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
    <p role="status" className="px-1 text-11 text-text-tertiary">
      {count} {count === 1 ? "update" : "updates"} waiting for {agentName}&apos;s next turn
    </p>
  );
}
