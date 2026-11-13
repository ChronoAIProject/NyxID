import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useAccountPanel } from "@/hooks/use-account-panel";
import { AssistantAccountMenu } from "@/components/assistant/assistant-account-menu";
import { agentOwnerSections } from "@/lib/assistant/nyxbot-labels";
import { useContext, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useShallow } from "zustand/react/shallow";
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
  Box,
  Building2,
  ChevronRight,
  ChevronsUpDown,
  FileText,
  Loader2,
  MoreHorizontal,
  MessageSquare,
  PencilLine,
  Plus,
  RefreshCw,
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
  useUiTranslation();
  return (
    <div className="shell-group-label">
      {children}
    </div>
  );
}

/** An owner's agents or crews: a top-level section label, flush with WORKSPACE. */
function OwnerLabel({ children }: { readonly children: ReactNode }) {
  useUiTranslation();
  return (
    <p className="-ml-2 shell-group-label">
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
  useUiTranslation();
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-disabled="true"
          data-keep-drawer-open=""
          className="shell-nav-item flex w-full cursor-not-allowed items-center text-left text-muted-foreground opacity-50"
        >
          <Icon className="h-4 w-4 shrink-0 text-text-tertiary" />
          <span className="min-w-0 flex-1 truncate">{label}</span>
          {trailing}
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{translateUi("Coming soon")}</TooltipContent>
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
  unread = false,
}: {
  readonly conversation: Conversation;
  readonly active: boolean;
  readonly ownerUserId: string | null;
  readonly onSelect: () => void;
  readonly onRequestDelete: () => void;
  readonly onRequestRename?: () => void;
  /** The thread has unread notifications. */
  readonly unread?: boolean;
}) {
  useUiTranslation();
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
        aria-label={unread ? translateUi("{{title}}, unread", { title: shownTitle }) : shownTitle}
        aria-describedby={showDraft ? draftPreviewId : undefined}
        className={cn(
          "w-full overflow-hidden px-2 py-1 text-left transition-colors",
          active
            ? "type-active-nav-item text-foreground"
            : "type-nav-item text-muted-foreground group-hover:text-foreground",
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
          {conversation.platform_thread && <span className="shrink-0 rounded bg-muted px-1 text-12 text-muted-foreground">{channelPlatformName(conversation.platform_thread.platform)}</span>}
          {unread ? (
            <span aria-hidden="true" data-unread-marker="" className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
          ) : null}
        </span>
        {showDraft && (
          <span
            id={draftPreviewId}
            className="mt-0.5 flex min-w-0 items-center gap-1 text-13 text-text-tertiary"
          >
            <PencilLine aria-hidden="true" className="h-2.5 w-2.5 shrink-0" />
            <span className="sr-only">{translateUi("Draft: ")}</span>
            <span className="min-w-0 flex-1 truncate">{draftPreview}</span>
          </span>
        )}
      </button>
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={translateUi("Options for {{value1}}", { value1: shownTitle })}
            data-icon-button=""
            data-keep-drawer-open=""
            className={cn(
              "absolute right-1 top-1 flex items-center justify-center rounded-md bg-card text-15 text-muted-foreground shadow-sm outline-none transition-opacity hover:bg-overlay-strong hover:text-foreground",
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
              <PencilLine aria-hidden="true" />{translateUi("Rename")}</DropdownMenuItem>
          ) : null}
          <DropdownMenuItem
            onSelect={onRequestDelete}
            className="text-destructive focus:text-destructive"
          >
            <Trash2 aria-hidden="true" />{translateUi("Delete")}</DropdownMenuItem>
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
  readonly destroyedTruncated?: boolean;
  readonly loading?: boolean;
  readonly loadError?: string;
  readonly onRetry?: () => void;
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
  /** Threads (including channel chats) with unread notifications. */
  readonly unreadThreadIds?: ReadonlySet<string>;
}

/** Group chats: the user plus several agents. */
export interface SidebarGroups {
  readonly groups: readonly AssistantGroup[];
  readonly selectedGroupId?: string;
  readonly loading?: boolean;
  readonly onSelectGroup: (groupId: string) => void;
  readonly onNewGroup: () => void;
  /** Crews with unread notifications; collaboration groups keep their own badge. */
  readonly unreadGroupIds?: ReadonlySet<string>;
}

function agentAccessibleName(agent: AssistantAgent): string {
  const title = agentTitle(agent);
  const handle = agentHandle(agent);
  const named = handle ? `${title} (${handle})` : title;
  if (agent.kind === "nyxbot") return translateUi("{{name}} — your personal agent", { name: named });
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
const AGENT_THREADS_SHOWN = 5;

/**
 * One channel bot's chats under its agent: collapsed to a single row (open
 * while it holds the open thread) so busy bots do not flood the sidebar.
 */
function ChannelThreadsGroup({
  group,
  activeThreadId,
  renderThread,
  unreadThreadIds,
}: {
  readonly group: ChannelThreadGroup;
  readonly activeThreadId: string | undefined;
  readonly renderThread: (conversation: Conversation) => ReactNode;
  readonly unreadThreadIds?: ReadonlySet<string>;
}) {
  useUiTranslation();
  const holdsActive = group.threads.some((thread) => thread.id === activeThreadId);
  // A click opens or closes the section for the thread open at that moment;
  // opening another of its chats opens it again.
  const [pinned, setPinned] = useState<{ open: boolean; active: string | undefined }>();
  const [shown, setShown] = useState(CHANNEL_THREADS_SHOWN);
  const open =
    pinned && (pinned.active === activeThreadId || !holdsActive) ? pinned.open : holdsActive;
  const running = group.threads.some((thread) => thread.active_turn);
  const unread = group.threads.some((thread) => unreadThreadIds?.has(thread.id));
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
        aria-label={translateUi("{{label}} on {{platform}}, {{count}} chats{{status}}", {
          label: group.label,
          platform,
          count,
          status: `${running ? translateUi(", working") : ""}${unread ? translateUi(", unread") : ""}`,
        })}
        onClick={() => setPinned({ open: !open, active: activeThreadId })}
        data-keep-drawer-open=""
        className="flex w-full items-center gap-1.5 rounded-lg py-1 pl-2 pr-2 text-left text-14 text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn("h-3 w-3 shrink-0 transition-transform", open && "rotate-90")}
        />
        <span className="min-w-0 flex-1 truncate">{group.label}</span>
        <span className="hidden shrink-0 rounded-md border border-hairline bg-overlay px-1 type-group-label text-text-tertiary @min-[15rem]:inline">
          {platform}
        </span>
        {running ? (
          <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
        ) : unread && !open ? (
          <span aria-hidden="true" data-unread-marker="" className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
        ) : null}
        <span className="shrink-0 tabular-nums text-13">{count}</span>
      </button>
      {open ? (
        // The guide line runs down from the chevron's centre.
        <div className="ml-3.5 space-y-px border-l border-border/60 pl-1">
          {Array.from(new Set(visible.map((thread) => thread.channel?.parent_chat_id ?? thread.channel?.chat_id ?? thread.id))).map((chatId) => {
            const children = visible.filter((thread) => (thread.channel?.parent_chat_id ?? thread.channel?.chat_id ?? thread.id) === chatId);
            const followed = children.some((thread) => thread.channel?.thread_id);
            return <div key={chatId}>
              {followed ? <p className="px-3 pt-1 text-13 text-text-tertiary">{children.find((thread) => thread.channel?.parent_title)?.channel?.parent_title ?? translateUi("Channel chat")}</p> : null}
              <div className={followed ? "ml-2 border-l border-hairline pl-1" : undefined}>{children.map(renderThread)}</div>
            </div>;
          })}
          {hidden > 0 ? (
            <button
              type="button"
              onClick={() => setShown((value) => value + CHANNEL_THREADS_SHOWN * 4)}
              data-keep-drawer-open=""
              className="w-full rounded-lg px-3 py-1 text-left text-13 text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
            >{translateUi("Show {{count}} more", { count: Math.min(hidden, CHANNEL_THREADS_SHOWN * 4) })}</button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** The @handle a display name hides, a hover away (agents keep one line). */
function HandleTooltip({ handle, children }: { readonly handle?: string; readonly children: ReactNode }) {
  useUiTranslation();
  if (!handle) return children;
  return (
    <TooltipProvider delayDuration={200}>
      <Tooltip>
        <TooltipTrigger asChild>{children}</TooltipTrigger>
        <TooltipContent side="bottom" align="start" className="text-13">{handle}</TooltipContent>
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
  useUiTranslation();
  const nyxbot = agent.kind === "nyxbot";
  // Clicking the selected agent folds its threads away for the thread open at
  // that moment; opening another of its threads unfolds them again.
  const [folded, setFolded] = useState<{ active: string | undefined }>();
  const expanded = selected && !(folded && folded.active === activeThreadId);
  const platforms = [...new Set(agent.channels.map((channel) => channel.platform))];
  const threads = expanded ? splitChannelThreads(model.threads) : { own: [], bots: [] };
  const [showAllThreads, setShowAllThreads] = useState(false);
  const threadListId = useId();
  const ownerUserId = useAuthStore((state) => state.user?.id ?? null);
  const draftedConversationIds = useAssistantDraftStore(
    useShallow((state) =>
      ownerUserId && state.ownerUserId === ownerUserId
        ? Object.entries(state.drafts)
            .filter(([key, draft]) => key.startsWith("conv:") && draft.text.trim())
            .map(([key]) => key.slice("conv:".length))
            .sort()
        : [],
    ),
  );
  const recentThreads = threads.own.filter(
    (conversation, index) =>
      index < AGENT_THREADS_SHOWN ||
      conversation.id === activeThreadId ||
      conversation.active_turn ||
      draftedConversationIds.includes(conversation.id),
  );
  const hiddenThreads = threads.own.length - recentThreads.length;
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
              "flex w-full items-center rounded-lg py-1 pl-3 pr-2 text-left transition-colors group-hover:bg-sidebar-hover",
              selected
                ? "type-active-nav-item text-foreground"
                : "type-nav-item text-muted-foreground group-hover:text-foreground",
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
                  className="hidden shrink-0 rounded-md border border-hairline bg-overlay px-1 type-group-label text-text-tertiary @min-[15rem]:inline"
                >
                  {channelPlatformName(platform)}
                  {platforms.length > 1 ? translateUi(" +{{value}}", { value: String(platforms.length - 1) }) : ""}
                </span>
              ))}
              {pending > 0 ? (
                <span className="shrink-0 rounded-md border border-warning/30 bg-warning/10 px-1.5 type-group-label text-warning">
                  {pending}
                </span>
              ) : null}
            </span>
          </button>
        </HandleTooltip>
        {hasMenu ? (
          <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
            <DropdownMenuTrigger asChild>
              <button data-icon-button=""
                ref={menuTrigger}
                type="button"
                aria-label={translateUi("Options for {{value1}}", { value1: name })}
                data-keep-drawer-open=""
                className={cn(
                  "absolute right-1 top-1/2 flex size-[1.5em] -translate-y-1/2 items-center justify-center rounded-md bg-card text-15 text-muted-foreground shadow-sm outline-none transition-opacity hover:bg-overlay-strong hover:text-foreground",
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
                  <Plus aria-hidden="true" />{translateUi("New chat")}</DropdownMenuItem>
              ) : null}
              {model.onAgentDetails ? (
                <DropdownMenuItem onSelect={() => model.onAgentDetails?.(agent.id)}>
                  <Settings2 aria-hidden="true" />{translateUi("Agent details")}</DropdownMenuItem>
              ) : null}
              {nyxbot ? (
                <DropdownMenuItem
                  onSelect={() => void openPanel("nyxbot", {}, menuTrigger.current ?? undefined)}
                >
                  <SlidersHorizontal aria-hidden="true" />{translateUi("NyxBot settings")}</DropdownMenuItem>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>
      {expanded ? (
        // The guide line runs down from the avatar's centre.
        <div
          role="group"
          aria-label={translateUi("Threads with {{value1}}", { value1: name })}
          className="mb-1 ml-[22px] mt-0.5 space-y-px border-l border-border/60 pl-1"
        >
          <div id={threadListId} className="space-y-px">
            {(showAllThreads ? threads.own : recentThreads).map(renderThread)}
          </div>
          {hiddenThreads > 0 ? (
            <button
              type="button"
              aria-expanded={showAllThreads}
              aria-controls={threadListId}
              data-keep-drawer-open=""
              onClick={() => setShowAllThreads((value) => !value)}
              className="w-full rounded-lg px-3 py-1 text-left text-13 text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-input-focus"
            >
              {showAllThreads ? translateUi("Show fewer") : translateUi("Show {{count}} more", { count: hiddenThreads })}
            </button>
          ) : null}
          {model.threadsLoading && !model.threads.length ? (
            <p className="px-3 py-1.5 text-13 text-text-tertiary">{translateUi("Loading threads...")}</p>
          ) : null}
          {threads.bots.map((group) => (
            <ChannelThreadsGroup
              key={group.key}
              group={group}
              activeThreadId={activeThreadId}
              renderThread={renderThread}
              unreadThreadIds={model.unreadThreadIds}
            />
          ))}
          {canChat ? (
            <button
              type="button"
              onClick={() => model.onNewThread(agent.id)}
              aria-label={translateUi("New chat with {{value1}}", { value1: name })}
              className="flex w-full items-center gap-2 rounded-lg px-2 py-1 text-left text-14 text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
            >
              <Plus aria-hidden="true" className="h-3 w-3" />{translateUi("New chat")}</button>
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
  useUiTranslation();
  const [showDestroyed, setShowDestroyed] = useState(false);
  const destroyed = model.agents.filter((agent) => agent.status === "destroyed");
  const visible = model.agents
    .filter((agent) => showDestroyed || agent.status !== "destroyed")
    // NyxBot pinned first; the server already orders the rest.
    .sort((a, b) => Number(b.kind === "nyxbot") - Number(a.kind === "nyxbot"));
  return (
    <div className="shell-nav-stack">
      <AgentsLoadState model={model} />
      {agentOwnerSections(visible).map((section) => (
        <section key={section.id} aria-label={translateUi("{{value1}} agents", { value1: section.label })}>
          <OwnerLabel>{section.label}{translateUi(" agents")}</OwnerLabel>
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
        <div>
          <button
            type="button"
            aria-pressed={showDestroyed}
            onClick={() => setShowDestroyed((value) => !value)}
            className="w-full rounded-lg px-3 py-1.5 text-left text-13 text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
          >
            {showDestroyed ? translateUi("Hide destroyed") : translateUi("Show destroyed ({{value1}})", { value1: String(destroyed.length) })}
          </button>
          {model.destroyedTruncated ? (
            <p className="px-3 pb-1.5 type-secondary text-text-tertiary">{translateUi("Older destroyed agents not shown.")}</p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** Keep a failed roster distinct from an empty one, including in the rail. */
function AgentsLoadState({ model, compact = false }: {
  readonly model: SidebarAgents;
  readonly compact?: boolean;
}) {
  useUiTranslation();
  if (model.loadError) {
    return compact ? (
      <RailItem label={translateUi("{{value1}} Retry", { value1: model.loadError })}>
        <button data-icon-button="" type="button" aria-label={translateUi("{{value1}} Retry", { value1: model.loadError })}
          onClick={model.onRetry} disabled={model.loading} className={cn(RAIL_BUTTON, "text-destructive")}>
          <RefreshCw aria-hidden="true" className="h-4 w-4" />
        </button>
      </RailItem>
    ) : (
      <div role="alert"><ErrorBanner message={model.loadError} onRetry={model.onRetry} /></div>
    );
  }
  if (model.agents.length) return null;
  const label = model.loading ? "Loading agents..." : "No agents to show.";
  return compact ? (
    <span role="status" aria-label={label} className={RAIL_BUTTON}>
      {model.loading ? <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" /> : null}
    </span>
  ) : <p role="status" className="px-3 py-1.5 text-13 text-text-tertiary">{label}</p>;
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
  useUiTranslation();
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          {compact ? (
            <button data-icon-button=""
              data-tour="create-agent"
              type="button"
              aria-label={translateUi("Create new agent")}
              title={translateUi("Create new agent")}
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
              data-tour="create-agent"
              className="mb-0.5 flex w-full items-center gap-3 rounded-lg px-3 py-1.5 text-left shell-nav-item text-muted-foreground transition-colors hover:bg-overlay hover:text-foreground data-[state=open]:bg-overlay data-[state=open]:text-foreground"
            >
              <Plus aria-hidden="true" className="h-4 w-4 shrink-0 text-text-tertiary" />
              <span className="truncate">{translateUi("Create new agent")}</span>
            </button>
          )}
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" side={compact ? "right" : undefined} className="min-w-[200px]">
          <DropdownMenuItem onSelect={() => agents.onNewAgent("personal")}>
            <User aria-hidden="true" />{translateUi("Personal agent")}</DropdownMenuItem>
          {agents.canCreateOrgAgent ? (
            <DropdownMenuItem onSelect={() => agents.onNewAgent("org")}>
              <Building2 aria-hidden="true" />{translateUi("Organization agent")}</DropdownMenuItem>
          ) : null}
          {onNewCrew ? (
            <DropdownMenuItem onSelect={onNewCrew}>
              <Users aria-hidden="true" />{translateUi("Crew")}</DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}

function GroupsSection({ model }: { readonly model: SidebarGroups }) {
  useUiTranslation();
  const sections = new Map<string, typeof model.groups>();
  for (const group of model.groups) {
    const key = group.owner?.type === "org" ? group.owner.id : "personal";
    sections.set(key, [...(sections.get(key) ?? []), group]);
  }
  // With no crews yet, the personal section still offers the first one.
  if (!sections.size) sections.set("personal", []);
  return (
    <div className="shell-nav-stack">
      {[...sections.entries()].map(([owner, groups]) => {
        const label = `${owner === "personal" ? "Personal" : (groups[0]?.owner?.name ?? "Organization")} crews`;
        return (
          <section key={owner} aria-label={label}>
            <OwnerLabel>{label}</OwnerLabel>
            {groups.map((group) => {
              const working = group.working_agent_ids.length;
              const selected = group.id === model.selectedGroupId;
              const unread = !group.collaboration && (model.unreadGroupIds?.has(group.id) ?? false);
              return (
                <button
                  key={group.id}
                  type="button"
                  onClick={() => model.onSelectGroup(group.id)}
                  aria-current={selected ? "page" : undefined}
                  aria-label={translateUi("{{value1}}, group with {{value2}}{{value3}}", { value1: group.name, value2: group.members
                    .map((member) => agentTitle(member))
                    .join(", "), value3: `${working ? translateUi(", {{value}} working", { value: working }) : ""}${unread ? translateUi(", unread") : ""}` })}
                  className={cn(
                    "group flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-left shell-nav-item transition-colors",
                    selected
                      ? "bg-overlay-strong font-medium text-foreground"
                      : "text-muted-foreground hover:bg-sidebar-hover hover:text-foreground",
                  )}
                >
                  <AgentAvatarStack agents={group.members} size="xs" max={3} ring="ring-sidebar group-hover:ring-sidebar-hover" />
                  <span className="min-w-0 flex-1 truncate">{group.name}</span>
                  {unread ? (
                    <span aria-hidden="true" data-unread-marker="" className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
                  ) : null}
                  <GroupUnread group={group} />
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
              <p className="px-3 py-1.5 text-13 text-text-tertiary">{translateUi("Loading crews...")}</p>
            ) : null}
            {owner === "personal" && !model.loading && !model.groups.length ? (
              <button
                type="button"
                onClick={model.onNewGroup}
                data-keep-drawer-open=""
                className="w-full rounded-lg px-3 py-1.5 text-left type-row-title text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
              >{translateUi("Chat with several agents at once")}</button>
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
    | "machines"
    | "sandboxes";
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
  useUiTranslation();
  const { current: accountPanel } = useAccountPanel();
  const rail = useContext(SidebarRailContext);
  const user = useAuthStore((state) => state.user);
  const counts = useAssistantWorkspaceCounts();
  const workspacePreview = useFeature(FEATURE_FLAG.ASSISTANT_WORKSPACE_PREVIEW);
  const cmaEnabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
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
        unread={agents?.unreadThreadIds?.has(conversation.id) ?? false}
      />
    );
  }

  if (rail) {
    return (
      <SidebarRail
        conversations={conversations}
        onNewChat={onNewChat}
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
            <Plus />{translateUi("New chat")}</Button>
        </div>
      )}

      <GroupLabel>{translateUi("Workspace")}</GroupLabel>
      <div className="shell-nav-stack px-2">
        {agents?.onHome ? (
          <button
            type="button"
            onClick={agents.onHome}
            aria-current={agents.homeActive ? "page" : undefined}
            className={`flex w-full items-center gap-3 rounded-lg px-3 py-1.5 text-left shell-nav-item transition-colors ${
              agents.homeActive
                ? "bg-overlay-strong font-medium text-foreground"
                : "text-muted-foreground hover:bg-overlay hover:text-foreground"
            }`}
          >
            <HomeGlyph
              className={`h-4 w-4 shrink-0 ${agents.homeActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
            />
            <span className="truncate">{translateUi("Home")}</span>
          </button>
        ) : (
          <Link
            to="/assistant"
            className="flex items-center gap-3 rounded-lg px-3 py-1.5 shell-nav-item text-muted-foreground hover:bg-overlay hover:text-foreground"
          >
            <HomeGlyph className="h-4 w-4" />{translateUi("Home")}</Link>
        )}
        {(
          [
            {
              view: "automations",
              to: "/assistant/automations",
              get label() { return translateUi("Automations"); },
              icon: AutomationsGlyph,
            },
            {
              view: "machines",
              to: "/assistant/machines",
              get label() { return translateUi("Machines"); },
              icon: MachinesGlyph,
            },
            ...(cmaEnabled ? [{ view: "sandboxes", to: "/assistant/sandboxes", label: "Sandboxes", icon: Box }] as const : []),
          ] as const
        ).map((item) => (
          <Link
            key={item.view}
            to={item.to}
            data-tour={item.view}
            aria-current={activeView === item.view ? "page" : undefined}
            className={cn(
              "flex w-full items-center gap-3 rounded-lg px-3 py-1.5 shell-nav-item transition-colors",
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
          className={`flex w-full items-center gap-3 rounded-lg px-3 py-1.5 shell-nav-item transition-colors ${
            pluginsActive
              ? "bg-overlay-strong font-medium text-foreground"
              : "text-muted-foreground hover:bg-overlay hover:text-foreground"
          }`}
        >
          <PluginsGlyph
            className={`h-4 w-4 shrink-0 ${pluginsActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
          />
          <span className="truncate">{translateUi("Plugins")}</span>
        </Link>
        {workspacePreview ? (
          <ComingSoonItem
            icon={FileText}
            label={translateUi("Artifacts")}
            trailing={
              <span className="font-mono text-12 text-text-tertiary">
                {counts.data?.artifacts ?? 0}
              </span>
            }
          />
        ) : null}
        <Link
          to="/assistant/approvals"
          data-tour="approvals"
          aria-current={approvalsActive ? "page" : undefined}
          className={`flex w-full items-center gap-3 rounded-lg px-3 py-1.5 shell-nav-item transition-colors ${
            approvalsActive
              ? "bg-overlay-strong font-medium text-foreground"
              : "text-muted-foreground hover:bg-overlay hover:text-foreground"
          }`}
        >
          <ApprovalsGlyph
            className={`h-4 w-4 shrink-0 ${approvalsActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
          />
          <span className="min-w-0 flex-1 truncate">{translateUi("Approvals")}</span>
          {(counts.data?.pendingApprovals ?? 0) > 0 && (
            <span className="rounded-md border border-warning/30 bg-warning/10 px-1.5 type-group-label text-warning">
              {counts.data?.pendingApprovals}
            </span>
          )}
        </Link>
        {workspacePreview ? (
          <ComingSoonItem icon={Activity} label={translateUi("Activity")} />
        ) : null}
      </div>

      {notice ? (
        <p
          role="status"
          className="mx-2 mb-2 rounded-md border border-border bg-overlay px-2.5 py-2 text-12 text-muted-foreground"
        >
          {notice}
        </p>
      ) : null}
      <nav aria-label={translateUi("Agents and conversations")} className="@container px-2 pb-3">
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
              <GroupLabel>{translateUi("Chats")}</GroupLabel>
            </div>
            <div className="shell-nav-stack">{conversations.map(renderRow)}</div>
          </>
        ) : null}
      </nav>
      </div>

      <div className="shrink-0 border-t border-border/60 p-2">
        <AssistantAccountMenu align="start" side="top">
          <button
            type="button"
            aria-label={translateUi("Account menu")}
            data-keep-drawer-open=""
            data-assistant-account-fallback="menu"
            className={cn(
              "shell-user-block w-full transition-colors hover:bg-overlay",
              Boolean(accountPanel.panel) && "bg-overlay-strong font-medium text-foreground",
            )}
          >
            <div className="shell-user-avatar border border-hairline bg-overlay-strong">
              <User className="h-3.5 w-3.5 text-text-tertiary" />
            </div>
            <div className="shell-user-copy">
              <p className="truncate shell-user-name">
                {user?.display_name ?? translateUi("User")}
              </p>
              <p className="truncate shell-user-email">
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
            <DialogTitle>{translateUi("Delete chat?")}</DialogTitle>
            <DialogDescription>
              {deleteTarget?.active_turn
                ? translateUi("The running reply will stop. “{{value1}}” and its history are removed permanently.", { value1: deleteTarget.title })
                : translateUi("“{{value1}}” and its history are removed permanently.", { value1: deleteTarget?.title })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => setDeleteTarget(undefined)}
            >{translateUi("Cancel")}</Button>
            <Button
              type="button"
              variant="destructive"
              isLoading={
                deleteTarget !== undefined &&
                (deletePendingIds.has(deleteTarget.id) ||
                  deleteTarget.id === deletingId)
              }
              onClick={() => void confirmDelete()}
            >{translateUi("Delete")}</Button>
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
  useUiTranslation();
  const form = useAppForm({
    resolver: zodResolver(nyxAgentTitleSchema),
    defaultValues: { title: conversation.title },
  });
  const [error, setError] = useState<string>();
  return (
        <form className="space-y-2 rounded-lg border border-hairline p-2" aria-label={translateUi("Rename chat")} onKeyDown={(event) => { if (event.key === "Escape") onClose(); }} onSubmit={form.handleSubmit(async ({ title }) => {
          try {
            await onRename(conversation.id, title);
            onClose();
          } catch {
            setError(translateUi("Could not rename this chat. Try again."));
          }
        })}>
          <Label htmlFor="chat-title">{translateUi("Title")}</Label>
          <Input autoFocus id="chat-title" maxLength={200} {...form.register("title")} />
          {error ? (
            <p role="alert" className="mt-2 text-14 text-destructive">
              {translateUi(error)}
            </p>
          ) : null}
          <div className="flex justify-end gap-1">
            <Button type="button" variant="ghost" onClick={onClose}>{translateUi("Cancel")}</Button>
            <Button
              type="submit"
              variant="primary"
              isLoading={form.formState.isSubmitting}
              disabled={!form.formState.isDirty || !form.watch("title").trim()}
            >{translateUi("Save")}</Button>
          </div>
        </form>
  );
}

const RAIL_BUTTON =
  "ui-icon-button relative inline-flex shrink-0 items-center justify-center rounded-lg text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring data-[state=open]:bg-overlay data-[state=open]:text-foreground";

/** One rail icon with its name in a tooltip. */
function RailItem({ label, children }: { readonly label: string; readonly children: ReactNode }) {
  useUiTranslation();
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The hidden sidebar: Workspace destinations as icons, then every agent as
 * an avatar (click lists its chats) and every crew, with the account last.
 */
function SidebarRail({
  conversations,
  onNewChat,
  agents,
  groups,
  activeView,
  activeConversationId,
  pendingApprovals,
  accountOpen,
  onSelect,
}: {
  readonly conversations: readonly Conversation[];
  readonly onNewChat: () => void;
  readonly agents?: SidebarAgents;
  readonly groups?: SidebarGroups;
  readonly activeView: "chat" | "plugins" | "approvals" | "automations" | "machines" | "sandboxes";
  readonly activeConversationId: string | undefined;
  readonly pendingApprovals: number;
  readonly accountOpen: boolean;
  readonly onSelect: (conversationId: string) => void;
}) {
  useUiTranslation();
  const homeActive = agents ? Boolean(agents.homeActive) : false;
  const cmaEnabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  const iconClass = (active: boolean) => cn("h-4 w-4", active && "text-nyx-secondary-400");
  const linkClass = (active: boolean) => cn(RAIL_BUTTON, active && "bg-overlay-strong text-foreground");
  const destinations = [
    { view: "automations", to: "/assistant/automations", get label() { return translateUi("Automations"); }, icon: AutomationsGlyph },
    { view: "machines", to: "/assistant/machines", get label() { return translateUi("Machines"); }, icon: MachinesGlyph },
    ...(cmaEnabled ? [{ view: "sandboxes", to: "/assistant/sandboxes", label: "Sandboxes", icon: Box }] as const : []),
    { view: "plugins", to: "/assistant/plugins", label: "Plugins", icon: PluginsGlyph },
  ] as const;
  const liveAgents = (agents?.agents ?? []).filter((agent) => agent.status !== "destroyed");
  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-full min-h-0 flex-col items-center bg-sidebar">
        <nav
          aria-label={translateUi("Workspace and agents")}
          className="shell-nav-stack min-h-0 w-full flex-1 items-center overflow-y-auto overscroll-contain py-1 [scrollbar-width:none]"
        >
          <RailItem label={translateUi("Home")}>
            {agents?.onHome ? (
              <button type="button" aria-label={translateUi("Home")} aria-current={homeActive ? "page" : undefined} onClick={agents.onHome} className={linkClass(homeActive)}>
                <HomeGlyph className={iconClass(homeActive)} />
              </button>
            ) : (
              <Link to="/assistant" aria-label={translateUi("Home")} className={linkClass(false)}>
                <HomeGlyph className="h-4 w-4" />
              </Link>
            )}
          </RailItem>
          {destinations.map((item) => (
            <RailItem key={item.view} label={item.label}>
              <Link
                to={item.to}
                data-tour={item.view}
                aria-label={item.label}
                aria-current={activeView === item.view ? "page" : undefined}
                className={linkClass(activeView === item.view)}
              >
                <item.icon className={iconClass(activeView === item.view)} />
              </Link>
            </RailItem>
          ))}
          <RailItem label={pendingApprovals ? translateUi("Approvals ({{value1}} waiting)", { value1: String(pendingApprovals) }) : translateUi("Approvals")}>
            <Link
              to="/assistant/approvals"
              data-tour="approvals"
              aria-label={pendingApprovals ? translateUi("Approvals, {{value1}} waiting", { value1: String(pendingApprovals) }) : translateUi("Approvals")}
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
          {agents ? <AgentsLoadState model={agents} compact /> : null}
          {!agents ? (
            <>
              <RailItem label={translateUi("New chat")}>
                <button data-icon-button="" type="button" aria-label={translateUi("New chat")} onClick={onNewChat} className={RAIL_BUTTON}><Plus className="h-4 w-4" /></button>
              </RailItem>
              <LegacyRailChats conversations={conversations} activeConversationId={activeConversationId} onSelect={onSelect} />
            </>
          ) : null}
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
            const unread = !group.collaboration && (groups.unreadGroupIds?.has(group.id) ?? false);
            return (
              <RailItem key={group.id} label={group.name}>
                <button
                  type="button"
                  aria-label={translateUi("{{value1}}, group{{status}}", { value1: group.name, status: unread ? translateUi(", unread") : "" })}
                  aria-current={selected ? "page" : undefined}
                  onClick={() => groups.onSelectGroup(group.id)}
                  className={linkClass(selected)}
                >
                  <AgentAvatarStack agents={group.members} size="xs" max={2} ring="ring-sidebar" />
                  {unread ? (
                    <span aria-hidden="true" data-unread-marker="" className="absolute top-0.5 right-0.5 h-1.5 w-1.5 rounded-full bg-primary" />
                  ) : null}
                  <GroupUnread group={group} compact />
                </button>
              </RailItem>
            );
          })}
        </nav>
        <div className="shrink-0 py-2">
          <AssistantAccountMenu align="start" side="top">
            <button data-icon-button=""
              type="button"
              aria-label={translateUi("Account menu")}
              data-assistant-account-fallback="menu"
              className={cn(
                "flex items-center justify-center rounded-lg border border-hairline bg-overlay-strong text-text-tertiary transition-colors hover:text-foreground",
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

function LegacyRailChats({ conversations, activeConversationId, onSelect }: {
  readonly conversations: readonly Conversation[];
  readonly activeConversationId: string | undefined;
  readonly onSelect: (conversationId: string) => void;
}) {
  useUiTranslation();
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button data-icon-button="" type="button" aria-label={translateUi("Previous chats")} title={translateUi("Previous chats")} className={RAIL_BUTTON}><MessageSquare className="h-4 w-4" /></button>
      </PopoverTrigger>
      <PopoverContent side="right" align="start" className="w-72 p-2">
        <p className="px-2 py-1 type-row-title">{translateUi("Previous chats")}</p>
        <ul aria-label={translateUi("Previous chats")} className="max-h-80 shell-nav-stack overflow-y-auto">
          {conversations.map((chat) => (
            <li key={chat.id}>
              <button type="button" aria-current={chat.id === activeConversationId ? "page" : undefined} onClick={() => { setOpen(false); onSelect(chat.id); }} className="w-full truncate rounded-md px-2 py-1.5 text-left text-14 text-muted-foreground hover:bg-overlay hover:text-foreground">{chat.title}</button>
            </li>
          ))}
        </ul>
        {!conversations.length ? <p className="px-2 py-1.5 text-13 text-text-tertiary">{translateUi("No chats yet.")}</p> : null}
      </PopoverContent>
    </Popover>
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
  useUiTranslation();
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
  useUiTranslation();
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
    <div className="space-y-1 text-14">
      <div className="flex items-center gap-2 px-1 pb-1">
        <AgentAvatar agent={agent} size="sm" />
        <span className="min-w-0 flex-1 truncate font-medium text-foreground">{agentTitle(agent)}</span>
        <button
          type="button"
          onClick={onNewChat}
          className="flex shrink-0 items-center gap-1 rounded-md px-1.5 py-0.5 type-row-title text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
        >
          <Plus aria-hidden="true" className="h-3 w-3" />{translateUi("New chat")}</button>
      </div>
      {loading && !chats.length ? (
        <p className="px-2 py-1.5 text-13 text-text-tertiary">{translateUi("Loading chats...")}</p>
      ) : !chats.length ? (
        <p className="px-2 py-1.5 text-13 text-text-tertiary">{translateUi("No chats yet.")}</p>
      ) : (
        <ul aria-label={translateUi("{{value1}} chats", { value1: agentTitle(agent) })} className="shell-nav-stack">
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
                {chat.title || translateUi("New conversation")}
              </button>
            </li>
          ))}
        </ul>
      )}
      {chats.length > RAIL_THREADS ? (
        <button
          type="button"
          onClick={onAll}
          className="w-full rounded-md px-2 py-1.5 text-left type-row-title text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
        >{translateUi("All chats (")}{chats.length})
        </button>
      ) : null}
    </div>
  );
}
function GroupUnread({ group, compact = false }: {
  readonly group: AssistantGroup;
  readonly compact?: boolean;
}) {
  useUiTranslation();
  const unread = group.collaboration?.unread_count ?? 0;
  const mentions = group.collaboration?.mention_count ?? 0;
  return (
    <>
      {unread > 0 ? (
        <span
          aria-label={translateUi("Unread messages")}
          className={cn("h-1.5 w-1.5 shrink-0 rounded-full bg-primary", compact && "absolute top-0.5 right-0.5")}
        />
      ) : null}
      {mentions > 0 ? (
        <span
          aria-label={translateUi("{{value1}} mentions", { value1: mentions })}
          className={cn("rounded-full bg-primary/10 px-1.5 type-group-label text-primary", compact && "absolute -bottom-0.5 right-0 bg-primary px-1 text-primary-foreground")}
        >
          @{mentions > 99 ? "99+" : mentions}
        </span>
      ) : null}
    </>
  );
}
