import { useAccountPanel } from "@/hooks/use-account-panel";
import { AssistantAccountMenu } from "@/components/assistant/assistant-account-menu";
import { agentOwnerSections } from "@/lib/assistant/nyxbot-labels";
import { useContext, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { SidebarRailContext } from "./sidebar-rail-context";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { nyxAgentTitleSchema } from "@/schemas/assistant-nyxagent";
import { isNyxAgentConversationId } from "@/lib/assistant/conversation-ids";
import { AgentStatusDot } from "@/components/assistant/nyxbot-agent-panels";
import { AgentAvatar, AgentAvatarStack } from "@/components/assistant/nyxbot-agent-avatar";
import {
  AGENT_STATUS_LABEL,
  agentHandle,
  agentTitle,
  channelPlatformName,
  splitChannelThreads,
  type ChannelThreadGroup,
} from "@/lib/assistant/nyxbot-labels";
import { ChatKindIcon } from "@/components/assistant/nyxbot-channel-chats";
import type { AssistantAgent, AssistantGroup } from "@/schemas/assistant-nyxagent";
import {
  Activity,
  Building2,
  ChevronRight,
  ChevronsUpDown,
  FileText,
  MoreHorizontal,
  PencilLine,
  Plus,
  Settings2,
  SlidersHorizontal,
  Trash2,
  User,
  Users,
  type LucideIcon,
} from "lucide-react";
import { Link } from "@tanstack/react-router";
import {
  ApprovalsGlyph,
  AutomationsGlyph,
  HomeGlyph,
  MachinesGlyph,
  PluginsGlyph,
} from "@/components/assistant/sidebar-nav-glyphs";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAssistantWorkspaceCounts } from "@/hooks/use-assistant-workspace";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { cn } from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import type { Conversation } from "@/types/assistant";

/**
 * Titles run to the very end of the column, so whenever the 3-dot is showing
 * it would otherwise sit on top of the text. Masking the tail (rather than
 * covering it with a chip) keeps the fade correct on every row background --
 * default, hover and active are all translucent overlays over `background`,
 * so no single gradient colour would match all three.
 */
const TITLE_FADE =
  "[mask-image:linear-gradient(to_right,#000_calc(100%_-_3rem),transparent_calc(100%_-_1.75rem))]";

function GroupLabel({ children }: { readonly children: string }) {
  return (
    <div className="px-3 py-2 text-9 font-medium uppercase tracking-[1.5px] text-text-tertiary">
      {children}
    </div>
  );
}

/** An owner's agents or crews: a top-level section label, flush with WORKSPACE. */
function OwnerLabel({ children }: { readonly children: ReactNode }) {
  return (
    <p className="-ml-2 px-3 py-2 text-9 font-medium uppercase tracking-[1.5px] text-text-tertiary">
      {children}
    </p>
  );
}

/** Workspace destination that is visible per the mockup but not yet built. */
function ComingSoonItem({
  icon: Icon,
  label,
  trailing,
}: {
  readonly icon: LucideIcon;
  readonly label: string;
  readonly trailing?: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          aria-disabled="true"
          className="flex w-full cursor-not-allowed items-center gap-3 rounded-lg px-3 py-1.5 text-13 text-muted-foreground opacity-50"
        >
          <Icon className="h-4 w-4 shrink-0 text-text-tertiary" />
          <span className="min-w-0 flex-1 truncate">{label}</span>
          {trailing}
        </div>
      </TooltipTrigger>
      <TooltipContent side="right">Coming soon</TooltipContent>
    </Tooltip>
  );
}

/**
 * One sidebar chat row: a full-width select button with the 3-dot menu laid
 * over its right edge, so the title always gets the whole column and the
 * trigger only takes space visually once the row is hovered.
 *
 * The always-on state is keyed on `(hover: none)`, not on a breakpoint: a
 * pointer-less device can be any width (an iPad is `md`), and app.css already
 * force-shows `group-hover:opacity-100` there. Revealing on width instead
 * would leave a wide tablet with a visible but `pointer-events: none` trigger
 * whose taps fall through to the title button underneath.
 *
 * The menu is a menu -- opening it must never look like a delete prompt --
 * and its Delete item raises the confirmation instead of deleting outright,
 * because the conversation and its history go permanently.
 */
function ConversationRow({
  conversation,
  active,
  ownerUserId,
  onSelect,
  onRequestDelete,
  onRequestRename,
}: {
  readonly conversation: Conversation;
  readonly active: boolean;
  readonly ownerUserId: string | null;
  readonly onSelect: () => void;
  readonly onRequestDelete: () => void;
  readonly onRequestRename?: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const draft = useAssistantDraftStore((state) =>
    ownerUserId && state.ownerUserId === ownerUserId
      ? (state.drafts[`conv:${conversation.id}`]?.text ?? "")
      : "",
  );
  const draftPreview = draft.replace(/\s+/g, " ").trim().slice(0, 80);
  // A chat app thread is named after its chat (group name, or the person).
  const shownTitle = conversation.channel?.chat_title ?? conversation.title;
  const showDraft = !active && draftPreview.length > 0;
  const draftPreviewId = `assistant-draft-${conversation.id}`;

  return (
    <div
      className={cn(
        "group relative flex items-center rounded-lg transition-colors",
        active ? "bg-overlay-strong" : "hover:bg-overlay",
      )}
    >
      <button
        type="button"
        onClick={onSelect}
        aria-label={shownTitle}
        aria-describedby={showDraft ? draftPreviewId : undefined}
        className={cn(
          "w-full overflow-hidden px-2 py-1 text-left text-13 transition-colors",
          active
            ? "font-medium text-foreground"
            : "text-muted-foreground group-hover:text-foreground",
          "[@media(hover:none)]:[mask-image:linear-gradient(to_right,#000_calc(100%_-_3rem),transparent_calc(100%_-_1.75rem))]",
          "group-hover:[mask-image:linear-gradient(to_right,#000_calc(100%_-_3rem),transparent_calc(100%_-_1.75rem))]",
          menuOpen && TITLE_FADE,
        )}
      >
        <span className="flex min-w-0 items-center gap-1.5">
          <ChatKindIcon
            kind={conversation.channel?.chat_kind}
            className="h-3 w-3 shrink-0 text-text-tertiary"
          />
          <span className="min-w-0 truncate">{shownTitle}</span>
          {conversation.platform_thread && <span className="shrink-0 rounded bg-muted px-1 text-10 text-muted-foreground">{channelPlatformName(conversation.platform_thread.platform)}</span>}
        </span>
        {showDraft && (
          <span
            id={draftPreviewId}
            className="mt-0.5 flex min-w-0 items-center gap-1 text-11 leading-4 text-text-tertiary"
          >
            <PencilLine aria-hidden="true" className="h-2.5 w-2.5 shrink-0" />
            <span className="sr-only">Draft: </span>
            <span className="min-w-0 flex-1 truncate">{draftPreview}</span>
          </span>
        )}
      </button>
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={`Options for ${shownTitle}`}
            data-keep-drawer-open=""
            className={cn(
              "absolute right-1 top-1 flex size-[1.5em] items-center justify-center rounded-md bg-card text-13 text-muted-foreground shadow-sm outline-none transition-opacity hover:bg-overlay-strong hover:text-foreground",
              "pointer-events-none opacity-0",
              "[@media(hover:none)]:pointer-events-auto [@media(hover:none)]:opacity-100",
              "group-hover:pointer-events-auto group-hover:opacity-100",
              "focus-visible:pointer-events-auto focus-visible:opacity-100",
              "data-[state=open]:pointer-events-auto data-[state=open]:opacity-100",
            )}
          >
            <MoreHorizontal className="size-[1.1em]" />
          </button>
        </DropdownMenuTrigger>
        {/* Above the z-[80] mobile sidebar drawer this can be opened from. */}
        <DropdownMenuContent align="end" className="min-w-[160px]">
          {onRequestRename ? (
            <DropdownMenuItem
              onSelect={onRequestRename}
            >
              <PencilLine aria-hidden="true" />
              Rename
            </DropdownMenuItem>
          ) : null}
          <DropdownMenuItem
            onSelect={onRequestDelete}
            className="text-destructive focus:text-destructive"
          >
            <Trash2 aria-hidden="true" />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

/**
 * The NyxBot agents section: NyxBot pinned first, then specialists. Its
 * presence switches the sidebar to the NyxAgent layout (Home, Agents,
 * Groups; no generic New chat and no legacy Chats list).
 */
export interface SidebarAgents {
  readonly agents: readonly AssistantAgent[];
  /** The agent whose threads are expanded. */
  readonly selectedAgentId?: string;
  /** The selected agent's threads, newest first. */
  readonly threads: readonly Conversation[];
  readonly threadsLoading?: boolean;
  readonly onSelectAgent: (agentId: string) => void;
  readonly onNewThread: (agentId: string) => void;
  /** Open an agent's details sheet; the row's hover menu offers it. */
  readonly onAgentDetails?: (agentId: string) => void;
  /** Create an agent: personal, or owned by an organization. */
  readonly onNewAgent: (owner: "personal" | "org") => void;
  /** Offer organization ownership in "Create agent". */
  readonly canCreateOrgAgent?: boolean;
  /** The NyxBot home is open. */
  readonly homeActive?: boolean;
  readonly onHome?: () => void;
}

/** Group chats: the user plus several agents. */
export interface SidebarGroups {
  readonly groups: readonly AssistantGroup[];
  readonly selectedGroupId?: string;
  readonly loading?: boolean;
  readonly onSelectGroup: (groupId: string) => void;
  readonly onNewGroup: () => void;
}

function agentAccessibleName(agent: AssistantAgent): string {
  const title = agentTitle(agent);
  const handle = agentHandle(agent);
  const named = handle ? `${title} (${handle})` : title;
  if (agent.kind === "nyxbot") return `${named} — your personal agent`;
  const pending = agent.pending_acknowledgements;
  return `${named}, specialist, ${AGENT_STATUS_LABEL[agent.status]}${
    pending ? `, ${String(pending)} pending ${pending === 1 ? "request" : "requests"}` : ""
  }`;
}

/**
 * One agent. Selecting it expands its threads beneath it with a
 * per-agent "New chat"; destroyed specialists are dimmed.
 */
/** Chats of one channel bot shown before "Show more". */
const CHANNEL_THREADS_SHOWN = 5;

/**
 * One channel bot's chats under its agent: collapsed to a single row (open
 * while it holds the open thread) so busy bots do not flood the sidebar.
 */
function ChannelThreadsGroup({
  group,
  activeThreadId,
  renderThread,
}: {
  readonly group: ChannelThreadGroup;
  readonly activeThreadId: string | undefined;
  readonly renderThread: (conversation: Conversation) => ReactNode;
}) {
  const holdsActive = group.threads.some((thread) => thread.id === activeThreadId);
  // A click opens or closes the section for the thread open at that moment;
  // opening another of its chats opens it again.
  const [pinned, setPinned] = useState<{ open: boolean; active: string | undefined }>();
  const [shown, setShown] = useState(CHANNEL_THREADS_SHOWN);
  const open =
    pinned && (pinned.active === activeThreadId || !holdsActive) ? pinned.open : holdsActive;
  const running = group.threads.some((thread) => thread.active_turn);
  const visible = group.threads.filter(
    (thread, index) => index < shown || thread.id === activeThreadId,
  );
  const hidden = group.threads.length - visible.length;
  const platform = channelPlatformName(group.platform);
  const count = group.threads.length;
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        aria-label={`${group.label} on ${platform}, ${String(count)} ${count === 1 ? "chat" : "chats"}${running ? ", working" : ""}`}
        onClick={() => setPinned({ open: !open, active: activeThreadId })}
        data-keep-drawer-open=""
        className="flex w-full items-center gap-1.5 rounded-lg py-1 pl-2 pr-2 text-left text-12 text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn("h-3 w-3 shrink-0 transition-transform", open && "rotate-90")}
        />
        <span className="min-w-0 flex-1 truncate">{group.label}</span>
        <span className="hidden shrink-0 rounded-md border border-hairline bg-overlay px-1 text-9 font-medium leading-4 text-text-tertiary @min-[15rem]:inline">
          {platform}
        </span>
        {running ? (
          <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
        ) : null}
        <span className="shrink-0 tabular-nums text-11">{count}</span>
      </button>
      {open ? (
        // The guide line runs down from the chevron's centre.
        <div className="ml-3.5 space-y-px border-l border-border/60 pl-1">
          {Array.from(new Set(visible.map((thread) => thread.channel?.parent_chat_id ?? thread.channel?.chat_id ?? thread.id))).map((chatId) => {
            const children = visible.filter((thread) => (thread.channel?.parent_chat_id ?? thread.channel?.chat_id ?? thread.id) === chatId);
            const followed = children.some((thread) => thread.channel?.thread_id);
            return <div key={chatId}>
              {followed ? <p className="px-3 pt-1 text-11 text-text-tertiary">{children.find((thread) => thread.channel?.parent_title)?.channel?.parent_title ?? "Channel chat"}</p> : null}
              <div className={followed ? "ml-2 border-l border-hairline pl-1" : undefined}>{children.map(renderThread)}</div>
            </div>;
          })}
          {hidden > 0 ? (
            <button
              type="button"
              onClick={() => setShown((value) => value + CHANNEL_THREADS_SHOWN * 4)}
              data-keep-drawer-open=""
              className="w-full rounded-lg px-3 py-1 text-left text-11 text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
            >
              Show {String(Math.min(hidden, CHANNEL_THREADS_SHOWN * 4))} more
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** The @handle a display name hides, a hover away (agents keep one line). */
function HandleTooltip({ handle, children }: { readonly handle?: string; readonly children: ReactNode }) {
  if (!handle) return children;
  return (
    <TooltipProvider delayDuration={200}>
      <Tooltip>
        <TooltipTrigger asChild>{children}</TooltipTrigger>
        <TooltipContent side="bottom" align="start" className="text-11">{handle}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

function AgentRow({
  agent,
  selected,
  model,
  activeThreadId,
  renderThread,
}: {
  readonly agent: AssistantAgent;
  readonly selected: boolean;
  readonly model: SidebarAgents;
  readonly activeThreadId: string | undefined;
  readonly renderThread: (conversation: Conversation) => ReactNode;
}) {
  const nyxbot = agent.kind === "nyxbot";
  // Clicking the selected agent folds its threads away for the thread open at
  // that moment; opening another of its threads unfolds them again.
  const [folded, setFolded] = useState<{ active: string | undefined }>();
  const expanded = selected && !(folded && folded.active === activeThreadId);
  const platforms = [...new Set(agent.channels.map((channel) => channel.platform))];
  const threads = expanded ? splitChannelThreads(model.threads) : { own: [], bots: [] };
  const pending = agent.pending_acknowledgements;
  const name = agentTitle(agent);
  const handle = agentHandle(agent);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  const sticky = useRef<HTMLDivElement>(null);
  // `data-stuck` marks the row while it is pinned (its threads scroll beneath
  // it). Only then does the list drop its top fade (app.css); otherwise rows
  // would end in a hard cut.
  useLayoutEffect(() => {
    const element = sticky.current;
    const scroller = element?.closest<HTMLElement>("[data-scroll-fade]");
    const block = element?.parentElement;
    if (!expanded || !element || !scroller || !block) return;
    const update = () => {
      const top = scroller.getBoundingClientRect().top;
      const box = block.getBoundingClientRect();
      element.toggleAttribute("data-stuck", box.top < top - 0.5 && box.bottom > top + element.offsetHeight);
    };
    update();
    scroller.addEventListener("scroll", update, { passive: true });
    return () => {
      scroller.removeEventListener("scroll", update);
      element.removeAttribute("data-stuck");
    };
  }, [expanded]);
  const { open: openPanel } = useAccountPanel();
  const canChat = agent.status !== "destroyed" && agent.can_use !== false;
  const hasMenu = canChat || nyxbot || Boolean(model.onAgentDetails);
  return (
    <div>
      {/* The open agent stays pinned to the top of the list while its threads
          scroll beneath it (app.css drops the list's top fade meanwhile). */}
      <div
        ref={sticky}
        data-sticky-agent={expanded ? "" : undefined}
        className={cn(
          "group relative",
          // Pinned, it fades the rows sliding under it instead of slicing them.
          expanded && "sticky top-0 z-10 bg-sidebar after:pointer-events-none after:absolute after:inset-x-0 after:top-full after:hidden after:h-4 after:bg-gradient-to-b after:from-sidebar after:to-transparent data-[stuck]:after:block",
        )}
      >
        <HandleTooltip handle={handle}>
          <button
            type="button"
            onClick={() => {
              if (selected) {
                setFolded(expanded ? { active: activeThreadId } : undefined);
              } else {
                setFolded(undefined);
                model.onSelectAgent(agent.id);
              }
            }}
            aria-label={agentAccessibleName(agent)}
            aria-expanded={expanded}
            // Folding is in place; keep the mobile drawer open for it.
            data-keep-drawer-open={selected ? "" : undefined}
            className={cn(
              // The whole row highlights (also while its options button is
              // hovered); only its content fades under that button.
              "flex w-full items-center rounded-lg py-1 pl-3 pr-2 text-left text-13 transition-colors group-hover:bg-sidebar-hover",
              selected
                ? "font-medium text-foreground"
                : "text-muted-foreground group-hover:text-foreground",
              agent.status === "destroyed" && "opacity-50",
            )}
          >
            <span
              className={cn(
                // -my-1/py-1 grow the masked box past the avatar, so the fade
                // never clips the status dot that overhangs it.
                "-my-1 flex min-w-0 flex-1 items-center gap-2 py-1",
                hasMenu && "[@media(hover:none)]:[mask-image:linear-gradient(to_right,#000_calc(100%_-_3rem),transparent_calc(100%_-_1.75rem))] group-hover:[mask-image:linear-gradient(to_right,#000_calc(100%_-_3rem),transparent_calc(100%_-_1.75rem))]",
                menuOpen && TITLE_FADE,
              )}
            >
              <span className="relative flex shrink-0">
                <AgentAvatar agent={agent} size="sm" />
                {nyxbot ? null : (
                  <AgentStatusDot
                    status={agent.status}
                    className="absolute -bottom-0.5 -right-0.5 ring-2 ring-sidebar transition-colors group-hover:ring-sidebar-hover"
                  />
                )}
              </span>
              <span className="flex min-w-0 flex-1 items-center gap-1">
                <span className="min-w-0 truncate">{name}</span>
                <ChevronRight
                  aria-hidden="true"
                  className={cn(
                    "h-3.5 w-3.5 shrink-0 text-text-tertiary transition-transform",
                    expanded && "rotate-90",
                  )}
                />
              </span>
              {/* Only once the sidebar is wide enough; the bot's chats name it anyway. */}
              {platforms.slice(0, 1).map((platform) => (
                <span
                  key={platform}
                  className="hidden shrink-0 rounded-md border border-hairline bg-overlay px-1 text-9 font-medium leading-4 text-text-tertiary @min-[15rem]:inline"
                >
                  {channelPlatformName(platform)}
                  {platforms.length > 1 ? ` +${String(platforms.length - 1)}` : ""}
                </span>
              ))}
              {pending > 0 ? (
                <span className="shrink-0 rounded-md border border-warning/30 bg-warning/10 px-1.5 text-10 font-medium text-warning">
                  {pending}
                </span>
              ) : null}
            </span>
          </button>
        </HandleTooltip>
        {hasMenu ? (
          <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
            <DropdownMenuTrigger asChild>
              <button
                ref={menuTrigger}
                type="button"
                aria-label={`Options for ${name}`}
                data-keep-drawer-open=""
                className={cn(
                  "absolute right-1 top-1/2 flex size-[1.5em] -translate-y-1/2 items-center justify-center rounded-md bg-card text-13 text-muted-foreground shadow-sm outline-none transition-opacity hover:bg-overlay-strong hover:text-foreground",
                  "pointer-events-none opacity-0",
                  "[@media(hover:none)]:pointer-events-auto [@media(hover:none)]:opacity-100",
                  "group-hover:pointer-events-auto group-hover:opacity-100",
                  "focus-visible:pointer-events-auto focus-visible:opacity-100",
                  "data-[state=open]:pointer-events-auto data-[state=open]:opacity-100",
                )}
              >
                <MoreHorizontal className="size-[1.1em]" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-[160px]">
              {canChat ? (
                <DropdownMenuItem onSelect={() => model.onNewThread(agent.id)}>
                  <Plus aria-hidden="true" />
                  New chat
                </DropdownMenuItem>
              ) : null}
              {model.onAgentDetails ? (
                <DropdownMenuItem onSelect={() => model.onAgentDetails?.(agent.id)}>
                  <Settings2 aria-hidden="true" />
                  Agent details
                </DropdownMenuItem>
              ) : null}
              {nyxbot ? (
                <DropdownMenuItem
                  onSelect={() => void openPanel("nyxbot", {}, menuTrigger.current ?? undefined)}
                >
                  <SlidersHorizontal aria-hidden="true" />
                  NyxBot settings
                </DropdownMenuItem>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>
      {expanded ? (
        // The guide line runs down from the avatar's centre.
        <div
          role="group"
          aria-label={`Threads with ${name}`}
          className="mb-1 ml-[22px] mt-0.5 space-y-px border-l border-border/60 pl-1"
        >
          {threads.own.map((conversation) => renderThread(conversation))}
          {model.threadsLoading && !model.threads.length ? (
            <p className="px-3 py-1.5 text-11 text-text-tertiary">
              Loading threads...
            </p>
          ) : null}
          {threads.bots.map((group) => (
            <ChannelThreadsGroup
              key={group.key}
              group={group}
              activeThreadId={activeThreadId}
              renderThread={renderThread}
            />
          ))}
          {canChat ? (
            <button
              type="button"
              onClick={() => model.onNewThread(agent.id)}
              aria-label={`New chat with ${name}`}
              className="flex w-full items-center gap-2 rounded-lg px-2 py-1 text-left text-12 text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
            >
              <Plus aria-hidden="true" className="h-3 w-3" />
              New chat
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function AgentsSection({
  model,
  activeThreadId,
  renderThread,
}: {
  readonly model: SidebarAgents;
  readonly activeThreadId: string | undefined;
  readonly renderThread: (conversation: Conversation) => ReactNode;
}) {
  const [showDestroyed, setShowDestroyed] = useState(false);
  const destroyed = model.agents.filter((agent) => agent.status === "destroyed");
  const visible = model.agents
    .filter((agent) => showDestroyed || agent.status !== "destroyed")
    // NyxBot pinned first; the server already orders the rest.
    .sort((a, b) => Number(b.kind === "nyxbot") - Number(a.kind === "nyxbot"));
  return (
    <div className="space-y-0.5">
      {agentOwnerSections(visible).map((section) => (
        <section key={section.id} aria-label={`${section.label} agents`}>
          <OwnerLabel>{section.label} agents</OwnerLabel>
          {section.agents.map((agent) => (
        <AgentRow
          key={agent.id}
          agent={agent}
          selected={agent.id === model.selectedAgentId}
          model={model}
          activeThreadId={activeThreadId}
          renderThread={renderThread}
        />
          ))}
        </section>
      ))}
      {destroyed.length ? (
        <button
          type="button"
          aria-pressed={showDestroyed}
          onClick={() => setShowDestroyed((value) => !value)}
          className="w-full rounded-lg px-3 py-1.5 text-left text-11 text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
        >
          {showDestroyed ? "Hide destroyed" : `Show destroyed (${String(destroyed.length)})`}
        </button>
      ) : null}
    </div>
  );
}

/**
 * The one place to create: a personal agent, an organization's agent, or a
 * crew (a group chat with several agents).
 */
function CreateAgentMenu({
  agents,
  onNewCrew,
  compact = false,
}: {
  readonly agents: SidebarAgents;
  readonly onNewCrew?: () => void;
  /** An icon button for the sidebar rail. */
  readonly compact?: boolean;
}) {
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          {compact ? (
            <button
              type="button"
              aria-label="Create new agent"
              title="Create new agent"
              className={RAIL_BUTTON}
            >
              <Plus aria-hidden="true" className="h-4 w-4" />
            </button>
          ) : (
            /* Styled like a Workspace destination. */
            <button
              type="button"
              // The dialogs are rendered by this sidebar; closing the mobile
              // drawer would unmount them.
              data-keep-drawer-open=""
              className="mb-0.5 flex w-full items-center gap-3 rounded-lg px-3 py-1.5 text-left text-13 text-muted-foreground transition-colors hover:bg-overlay hover:text-foreground data-[state=open]:bg-overlay data-[state=open]:text-foreground"
            >
              <Plus aria-hidden="true" className="h-4 w-4 shrink-0 text-text-tertiary" />
              <span className="truncate">Create new agent</span>
            </button>
          )}
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" side={compact ? "right" : undefined} className="min-w-[200px]">
          <DropdownMenuItem onSelect={() => agents.onNewAgent("personal")}>
            <User aria-hidden="true" />
            Personal agent
          </DropdownMenuItem>
          {agents.canCreateOrgAgent ? (
            <DropdownMenuItem onSelect={() => agents.onNewAgent("org")}>
              <Building2 aria-hidden="true" />
              Organization agent
            </DropdownMenuItem>
          ) : null}
          {onNewCrew ? (
            <DropdownMenuItem onSelect={onNewCrew}>
              <Users aria-hidden="true" />
              Crew
            </DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}

function GroupsSection({ model }: { readonly model: SidebarGroups }) {
  const sections = new Map<string, typeof model.groups>();
  for (const group of model.groups) {
    const key = group.owner?.type === "org" ? group.owner.id : "personal";
    sections.set(key, [...(sections.get(key) ?? []), group]);
  }
  // With no crews yet, the personal section still offers the first one.
  if (!sections.size) sections.set("personal", []);
  return (
    <div className="space-y-0.5">
      {[...sections.entries()].map(([owner, groups]) => {
        const label = `${owner === "personal" ? "Personal" : (groups[0]?.owner?.name ?? "Organization")} crews`;
        return (
          <section key={owner} aria-label={label}>
            <OwnerLabel>{label}</OwnerLabel>
            {groups.map((group) => {
              const working = group.working_agent_ids.length;
              const selected = group.id === model.selectedGroupId;
              return (
                <button
                  key={group.id}
                  type="button"
                  onClick={() => model.onSelectGroup(group.id)}
                  aria-current={selected ? "page" : undefined}
                  aria-label={`${group.name}, group with ${group.members
                    .map((member) => agentTitle(member))
                    .join(", ")}${working ? `, ${String(working)} working` : ""}`}
                  className={cn(
                    "group flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-left text-13 transition-colors",
                    selected
                      ? "bg-overlay-strong font-medium text-foreground"
                      : "text-muted-foreground hover:bg-sidebar-hover hover:text-foreground",
                  )}
                >
                  <AgentAvatarStack agents={group.members} size="xs" max={3} ring="ring-sidebar group-hover:ring-sidebar-hover" />
                  <span className="min-w-0 flex-1 truncate">{group.name}</span>
                  {working ? (
                    <span
                      aria-hidden="true"
                      className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-success"
                    />
                  ) : null}
                </button>
              );
            })}
            {owner === "personal" && model.loading && !model.groups.length ? (
              <p className="px-3 py-1.5 text-11 text-text-tertiary">Loading crews...</p>
            ) : null}
            {owner === "personal" && !model.loading && !model.groups.length ? (
              <button
                type="button"
                onClick={model.onNewGroup}
                data-keep-drawer-open=""
                className="w-full rounded-lg px-3 py-1.5 text-left text-11 text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
              >
                Chat with several agents at once
              </button>
            ) : null}
          </section>
        );
      })}
    </div>
  );
}

export function AssistantSidebar({
  conversations,
  activeConversationId,
  activeView = "chat",
  deletingId,
  notice,
  onNewChat,
  onSelect,
  onDelete,
  onRename,
  agents,
  groups,
}: {
  readonly conversations: readonly Conversation[];
  readonly activeConversationId: string | undefined;
  readonly activeView?:
    | "chat"
    | "plugins"
    | "approvals"
    | "automations"
    | "machines";
  readonly deletingId?: string;
  readonly notice?: string;
  readonly onNewChat: () => void;
  readonly onSelect: (conversationId: string) => void;
  readonly onDelete: (conversationId: string) => void | Promise<void>;
  readonly onRename?: (conversationId: string, title: string) => Promise<void>;
  /** NyxBot agents and the selected agent's threads (NyxAgent engine). */
  readonly agents?: SidebarAgents;
  /** NyxBot group chats (NyxAgent engine). */
  readonly groups?: SidebarGroups;
}) {
  const { current: accountPanel } = useAccountPanel();
  const rail = useContext(SidebarRailContext);
  const user = useAuthStore((state) => state.user);
  const counts = useAssistantWorkspaceCounts();
  const workspacePreview = useFeature(FEATURE_FLAG.ASSISTANT_WORKSPACE_PREVIEW);
  const pluginsActive = activeView === "plugins";
  const approvalsActive = activeView === "approvals";
  const [deleteTarget, setDeleteTarget] = useState<Conversation | undefined>();
  const [renameTarget, setRenameTarget] = useState<Conversation | undefined>();
  const [deletePendingIds, setDeletePendingIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );

  // One dialog for the whole list rather than one per row. A rejected delete
  // has already been said out loud by the caller, so the dialog stays open
  // and the action remains retryable; a resolved one closes it (the row is
  // gone from the list by then anyway).
  //
  // Dismissal stays available at all times -- `apiClient` has no timeout, so
  // holding the dialog shut around a hung request would trap the user -- so a
  // request can outlive the dialog that started it, and several can be in
  // flight against different chats at once. Hence a set of pending ids rather
  // than one marker (a scalar would forget chat A the moment B was submitted,
  // and let A be submitted twice), and hence every read and write below keyed
  // on the id captured at submit rather than on whatever is open now.
  async function confirmDelete() {
    const target = deleteTarget;
    if (!target || deletePendingIds.has(target.id)) return;
    setDeletePendingIds((current) => new Set(current).add(target.id));
    try {
      await onDelete(target.id);
      setDeleteTarget((current) =>
        current?.id === target.id ? undefined : current,
      );
    } catch {
      /* keep the dialog open so Delete can be pressed again */
    } finally {
      setDeletePendingIds((current) => {
        const next = new Set(current);
        next.delete(target.id);
        return next;
      });
    }
  }

  function renderRow(conversation: Conversation) {
    if (renameTarget?.id === conversation.id && onRename) {
      return <RenameChatInline key={conversation.id} conversation={conversation}
        onClose={() => setRenameTarget(undefined)} onRename={onRename} />;
    }
    return (
      <ConversationRow
        key={conversation.id}
        conversation={conversation}
        active={conversation.id === activeConversationId}
        ownerUserId={user?.id ?? null}
        onSelect={() => onSelect(conversation.id)}
        onRequestDelete={() => setDeleteTarget(conversation)}
        onRequestRename={onRename && isNyxAgentConversationId(conversation.id)
          ? () => setRenameTarget(conversation)
          : undefined}
      />
    );
  }

  if (rail) {
    return (
      <SidebarRail
        agents={agents}
        groups={groups}
        activeView={activeView}
        activeConversationId={activeConversationId}
        pendingApprovals={counts.data?.pendingApprovals ?? 0}
        accountOpen={Boolean(accountPanel.panel)}
        onSelect={onSelect}
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-sidebar">
      {/* One scroll area for the whole column; the account button stays put. */}
      {/* Always a visible scrollbar, so the column reads as scrollable. */}
      <div data-scroll-fade="" className="min-h-0 flex-1 overflow-y-scroll overscroll-contain">
      {/* NyxAgent mode starts chats from the NyxBot home and each agent's
          own "New chat"; the generic button belongs to the earlier engines. */}
      {agents ? (
        <div className="pt-1" />
      ) : (
        <div className="p-2.5">
          {/* No loading state: "New chat" is navigation only — it issues no
              requests. The conversation is allocated lazily by the first send. */}
          <Button
            type="button"
            variant="primary"
            className="w-full"
            onClick={onNewChat}
          >
            <Plus />
            New chat
          </Button>
        </div>
      )}

      <GroupLabel>Workspace</GroupLabel>
      <div className="space-y-0.5 px-2">
        {agents?.onHome ? (
          <button
            type="button"
            onClick={agents.onHome}
            aria-current={agents.homeActive ? "page" : undefined}
            className={`flex w-full items-center gap-3 rounded-lg px-3 py-1.5 text-left text-13 transition-colors ${
              agents.homeActive
                ? "bg-overlay-strong font-medium text-foreground"
                : "text-muted-foreground hover:bg-overlay hover:text-foreground"
            }`}
          >
            <HomeGlyph
              className={`h-4 w-4 shrink-0 ${agents.homeActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
            />
            <span className="truncate">Home</span>
          </button>
        ) : (
          <Link
            to="/assistant"
            className="flex items-center gap-3 rounded-lg px-3 py-1.5 text-13 text-muted-foreground hover:bg-overlay hover:text-foreground"
          >
            <HomeGlyph className="h-4 w-4" />
            Home
          </Link>
        )}
        {(
          [
            {
              view: "automations",
              to: "/assistant/automations",
              label: "Automations",
              icon: AutomationsGlyph,
            },
            {
              view: "machines",
              to: "/assistant/machines",
              label: "Machines",
              icon: MachinesGlyph,
            },
          ] as const
        ).map((item) => (
          <Link
            key={item.view}
            to={item.to}
            aria-current={activeView === item.view ? "page" : undefined}
            className={cn(
              "flex w-full items-center gap-3 rounded-lg px-3 py-1.5 text-13 transition-colors",
              activeView === item.view
                ? "bg-overlay-strong font-medium text-foreground"
                : "text-muted-foreground hover:bg-overlay hover:text-foreground",
            )}
          >
            <item.icon
              className={cn(
                "h-4 w-4 shrink-0",
                activeView === item.view
                  ? "text-nyx-secondary-400"
                  : "text-text-tertiary",
              )}
            />
            <span className="truncate">{item.label}</span>
          </Link>
        ))}
        <Link
          to="/assistant/plugins"
          aria-current={pluginsActive ? "page" : undefined}
          className={`flex w-full items-center gap-3 rounded-lg px-3 py-1.5 text-13 transition-colors ${
            pluginsActive
              ? "bg-overlay-strong font-medium text-foreground"
              : "text-muted-foreground hover:bg-overlay hover:text-foreground"
          }`}
        >
          <PluginsGlyph
            className={`h-4 w-4 shrink-0 ${pluginsActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
          />
          <span className="truncate">Plugins</span>
        </Link>
        {workspacePreview ? (
          <ComingSoonItem
            icon={FileText}
            label="Artifacts"
            trailing={
              <span className="font-mono text-9 text-text-tertiary">
                {counts.data?.artifacts ?? 0}
              </span>
            }
          />
        ) : null}
        <Link
          to="/assistant/approvals"
          aria-current={approvalsActive ? "page" : undefined}
          className={`flex w-full items-center gap-3 rounded-lg px-3 py-1.5 text-13 transition-colors ${
            approvalsActive
              ? "bg-overlay-strong font-medium text-foreground"
              : "text-muted-foreground hover:bg-overlay hover:text-foreground"
          }`}
        >
          <ApprovalsGlyph
            className={`h-4 w-4 shrink-0 ${approvalsActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
          />
          <span className="min-w-0 flex-1 truncate">Approvals</span>
          {(counts.data?.pendingApprovals ?? 0) > 0 && (
            <span className="rounded-md border border-warning/30 bg-warning/10 px-1.5 text-10 font-medium text-warning">
              {counts.data?.pendingApprovals}
            </span>
          )}
        </Link>
        {workspacePreview ? (
          <ComingSoonItem icon={Activity} label="Activity" />
        ) : null}
      </div>

      {notice ? (
        <p
          role="status"
          className="mx-2 mb-2 rounded-md border border-border bg-overlay px-2.5 py-2 text-10 leading-relaxed text-muted-foreground"
        >
          {notice}
        </p>
      ) : null}
      <nav aria-label="Agents and conversations" className="@container px-2 pb-3">
        {agents ? (
          <>
            <CreateAgentMenu agents={agents} onNewCrew={groups?.onNewGroup} />
            <AgentsSection
              model={agents}
              activeThreadId={activeConversationId}
              renderThread={renderRow}
            />
          </>
        ) : null}
        {groups ? <GroupsSection model={groups} /> : null}
        {/* The earlier engines' chats; NyxAgent mode lists threads under agents. */}
        {!agents ? (
          <>
            <div className="-mx-2">
              <GroupLabel>Chats</GroupLabel>
            </div>
            <div className="space-y-0.5">{conversations.map(renderRow)}</div>
          </>
        ) : null}
      </nav>
      </div>

      <div className="shrink-0 border-t border-border/60 p-2">
        <AssistantAccountMenu align="start" side="top">
          <button
            type="button"
            aria-label="Account menu"
            data-keep-drawer-open=""
            data-assistant-account-fallback="menu"
            className={cn(
              "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors hover:bg-overlay",
              Boolean(accountPanel.panel) && "bg-overlay-strong font-medium text-foreground",
            )}
          >
            <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay-strong">
              <User className="h-3.5 w-3.5 text-text-tertiary" />
            </div>
            <div className="min-w-0 flex-1">
              <p className="truncate text-12 font-medium text-foreground">
                {user?.display_name ?? "User"}
              </p>
              <p className="truncate text-10 text-text-tertiary">
                {user?.email ?? ""}
              </p>
            </div>
            <ChevronsUpDown aria-hidden="true" className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
          </button>
        </AssistantAccountMenu>
      </div>

      <Dialog
        open={deleteTarget !== undefined}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(undefined);
        }}
      >
        <DialogContent className="md:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete chat?</DialogTitle>
            <DialogDescription>
              {deleteTarget?.active_turn
                ? `The running reply will stop. “${deleteTarget.title}” and its history are removed permanently.`
                : `“${deleteTarget?.title}” and its history are removed permanently.`}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => setDeleteTarget(undefined)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant="destructive"
              isLoading={
                deleteTarget !== undefined &&
                (deletePendingIds.has(deleteTarget.id) ||
                  deleteTarget.id === deletingId)
              }
              onClick={() => void confirmDelete()}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function RenameChatInline({
  conversation,
  onClose,
  onRename,
}: {
  readonly conversation: Conversation;
  readonly onClose: () => void;
  readonly onRename: (id: string, title: string) => Promise<void>;
}) {
  const form = useAppForm({
    resolver: zodResolver(nyxAgentTitleSchema),
    defaultValues: { title: conversation.title },
  });
  const [error, setError] = useState<string>();
  return (
        <form className="space-y-2 rounded-lg border border-hairline p-2" aria-label="Rename chat" onKeyDown={(event) => { if (event.key === "Escape") onClose(); }} onSubmit={form.handleSubmit(async ({ title }) => {
          try {
            await onRename(conversation.id, title);
            onClose();
          } catch {
            setError("Could not rename this chat. Try again.");
          }
        })}>
          <label htmlFor="chat-title" className="text-12">
            Title
          </label>
          <Input autoFocus id="chat-title" maxLength={200} {...form.register("title")} />
          {error ? (
            <p role="alert" className="mt-2 text-12 text-destructive">
              {error}
            </p>
          ) : null}
          <div className="flex justify-end gap-1">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="primary"
              isLoading={form.formState.isSubmitting}
              disabled={!form.formState.isDirty || !form.watch("title").trim()}
            >
              Save
            </Button>
          </div>
        </form>
  );
}

const RAIL_BUTTON =
  "relative flex h-9 w-9 shrink-0 items-center justify-center rounded-lg text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring data-[state=open]:bg-overlay data-[state=open]:text-foreground";

/** One rail icon with its name in a tooltip. */
function RailItem({ label, children }: { readonly label: string; readonly children: ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The hidden sidebar: Workspace destinations as icons, then every agent as
 * an avatar (hover lists its chats) and every crew, with the account last.
 */
function SidebarRail({
  agents,
  groups,
  activeView,
  activeConversationId,
  pendingApprovals,
  accountOpen,
  onSelect,
}: {
  readonly agents?: SidebarAgents;
  readonly groups?: SidebarGroups;
  readonly activeView: "chat" | "plugins" | "approvals" | "automations" | "machines";
  readonly activeConversationId: string | undefined;
  readonly pendingApprovals: number;
  readonly accountOpen: boolean;
  readonly onSelect: (conversationId: string) => void;
}) {
  const homeActive = agents ? Boolean(agents.homeActive) : false;
  const iconClass = (active: boolean) => cn("h-4 w-4", active && "text-nyx-secondary-400");
  const linkClass = (active: boolean) => cn(RAIL_BUTTON, active && "bg-overlay-strong text-foreground");
  const destinations = [
    { view: "automations", to: "/assistant/automations", label: "Automations", icon: AutomationsGlyph },
    { view: "machines", to: "/assistant/machines", label: "Machines", icon: MachinesGlyph },
    { view: "plugins", to: "/assistant/plugins", label: "Plugins", icon: PluginsGlyph },
  ] as const;
  const liveAgents = (agents?.agents ?? []).filter((agent) => agent.status !== "destroyed");
  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-full min-h-0 flex-col items-center bg-sidebar">
        <nav
          aria-label="Workspace and agents"
          className="flex min-h-0 w-full flex-1 flex-col items-center gap-1 overflow-y-auto overscroll-contain py-1 [scrollbar-width:none]"
        >
          <RailItem label="Home">
            {agents?.onHome ? (
              <button type="button" aria-label="Home" aria-current={homeActive ? "page" : undefined} onClick={agents.onHome} className={linkClass(homeActive)}>
                <HomeGlyph className={iconClass(homeActive)} />
              </button>
            ) : (
              <Link to="/assistant" aria-label="Home" className={linkClass(false)}>
                <HomeGlyph className="h-4 w-4" />
              </Link>
            )}
          </RailItem>
          {destinations.map((item) => (
            <RailItem key={item.view} label={item.label}>
              <Link
                to={item.to}
                aria-label={item.label}
                aria-current={activeView === item.view ? "page" : undefined}
                className={linkClass(activeView === item.view)}
              >
                <item.icon className={iconClass(activeView === item.view)} />
              </Link>
            </RailItem>
          ))}
          <RailItem label={pendingApprovals ? `Approvals (${String(pendingApprovals)} waiting)` : "Approvals"}>
            <Link
              to="/assistant/approvals"
              aria-label={pendingApprovals ? `Approvals, ${String(pendingApprovals)} waiting` : "Approvals"}
              aria-current={activeView === "approvals" ? "page" : undefined}
              className={linkClass(activeView === "approvals")}
            >
              <ApprovalsGlyph className={iconClass(activeView === "approvals")} />
              {pendingApprovals ? (
                <span aria-hidden="true" className="absolute top-1.5 right-1.5 h-1.5 w-1.5 rounded-full bg-warning" />
              ) : null}
            </Link>
          </RailItem>
          {agents ? <CreateAgentMenu agents={agents} onNewCrew={groups?.onNewGroup} compact /> : null}
          {liveAgents.length ? <div className="h-2 shrink-0" aria-hidden="true" /> : null}
          {agents
            ? liveAgents.map((agent) => (
                <RailAgent
                  key={agent.id}
                  agent={agent}
                  model={agents}
                  activeConversationId={activeConversationId}
                  onSelect={onSelect}
                />
              ))
            : null}
          {groups?.groups.length ? <div className="h-2 shrink-0" aria-hidden="true" /> : null}
          {groups?.groups.map((group) => {
            const selected = group.id === groups.selectedGroupId;
            return (
              <RailItem key={group.id} label={group.name}>
                <button
                  type="button"
                  aria-label={`${group.name}, group`}
                  aria-current={selected ? "page" : undefined}
                  onClick={() => groups.onSelectGroup(group.id)}
                  className={linkClass(selected)}
                >
                  <AgentAvatarStack agents={group.members} size="xs" max={2} ring="ring-sidebar" />
                </button>
              </RailItem>
            );
          })}
        </nav>
        <div className="shrink-0 py-2">
          <AssistantAccountMenu align="start" side="top">
            <button
              type="button"
              aria-label="Account menu"
              data-assistant-account-fallback="menu"
              className={cn(
                "flex h-8 w-8 items-center justify-center rounded-lg border border-hairline bg-overlay-strong text-text-tertiary transition-colors hover:text-foreground",
                accountOpen && "text-foreground",
              )}
            >
              <User aria-hidden="true" className="h-3.5 w-3.5" />
            </button>
          </AssistantAccountMenu>
        </div>
      </div>
    </TooltipProvider>
  );
}

/** Threads shown in an agent's hover card before "All chats". */
const RAIL_THREADS = 8;

/** An agent on the rail: a click lists its chats, with "All chats" to open the agent. */
function RailAgent({
  agent,
  model,
  activeConversationId,
  onSelect,
}: {
  readonly agent: AssistantAgent;
  readonly model: SidebarAgents;
  readonly activeConversationId: string | undefined;
  readonly onSelect: (conversationId: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const selected = agent.id === model.selectedAgentId;
  return (
    // Its chats open on click, never on a passing pointer; "All chats"
    // inside opens the agent itself.
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={agentAccessibleName(agent)}
          aria-current={selected ? "true" : undefined}
          className={cn(RAIL_BUTTON, selected && "bg-overlay-strong")}
        >
          <AgentAvatar agent={agent} size="md" />
          {agent.status === "running" ? (
            <span aria-hidden="true" className="absolute right-1 bottom-1 h-1.5 w-1.5 rounded-full bg-success motion-safe:animate-pulse" />
          ) : null}
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="right"
        align="start"
        sideOffset={8}
        className="w-64 p-2"
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        {open ? (
          <RailAgentChats
            agent={agent}
            model={model}
            activeConversationId={activeConversationId}
            onPick={(conversationId) => {
              setOpen(false);
              onSelect(conversationId);
            }}
            onNewChat={() => {
              setOpen(false);
              model.onNewThread(agent.id);
            }}
            onAll={() => {
              setOpen(false);
              model.onSelectAgent(agent.id);
            }}
          />
        ) : null}
      </PopoverContent>
    </Popover>
  );
}

function RailAgentChats({
  agent,
  model,
  activeConversationId,
  onPick,
  onNewChat,
  onAll,
}: {
  readonly agent: AssistantAgent;
  readonly model: SidebarAgents;
  readonly activeConversationId: string | undefined;
  readonly onPick: (conversationId: string) => void;
  readonly onNewChat: () => void;
  readonly onAll: () => void;
}) {
  const userId = useAuthStore((state) => state.user?.id);
  const selected = agent.id === model.selectedAgentId;
  // The open agent's threads are already loaded; others load (and cache) on hover.
  const threads = useQuery({
    queryKey: nyxBotQueryKeys.threads(userId, agent.id),
    queryFn: () => nyxAgentTransport.list(agent.id),
    enabled: !selected && Boolean(userId),
  });
  const chats = selected ? model.threads : (threads.data ?? []);
  const loading = selected ? Boolean(model.threadsLoading) : threads.isPending;
  return (
    <div className="space-y-1 text-12">
      <div className="flex items-center gap-2 px-1 pb-1">
        <AgentAvatar agent={agent} size="sm" />
        <span className="min-w-0 flex-1 truncate font-medium text-foreground">{agentTitle(agent)}</span>
        <button
          type="button"
          onClick={onNewChat}
          className="flex shrink-0 items-center gap-1 rounded-md px-1.5 py-0.5 text-11 text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
        >
          <Plus aria-hidden="true" className="h-3 w-3" />
          New chat
        </button>
      </div>
      {loading && !chats.length ? (
        <p className="px-2 py-1.5 text-11 text-text-tertiary">Loading chats...</p>
      ) : !chats.length ? (
        <p className="px-2 py-1.5 text-11 text-text-tertiary">No chats yet.</p>
      ) : (
        <ul aria-label={`${agentTitle(agent)} chats`} className="space-y-0.5">
          {chats.slice(0, RAIL_THREADS).map((chat) => (
            <li key={chat.id}>
              <button
                type="button"
                onClick={() => onPick(chat.id)}
                aria-current={chat.id === activeConversationId ? "page" : undefined}
                className={cn(
                  "w-full truncate rounded-md px-2 py-1.5 text-left transition-colors hover:bg-overlay hover:text-foreground",
                  chat.id === activeConversationId ? "bg-overlay-strong text-foreground" : "text-muted-foreground",
                )}
              >
                {chat.title || "New conversation"}
              </button>
            </li>
          ))}
        </ul>
      )}
      {chats.length > RAIL_THREADS ? (
        <button
          type="button"
          onClick={onAll}
          className="w-full rounded-md px-2 py-1.5 text-left text-11 text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
        >
          All chats ({chats.length})
        </button>
      ) : null}
    </div>
  );
}
