import { agentOwnerSections } from "@/lib/assistant/nyxbot-labels";
import { useState, type ReactNode } from "react";
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
  ChevronRight,
  FileText,
  House,
  LayoutGrid,
  MoreHorizontal,
  PencilLine,
  Plus,
  Monitor,
  CalendarClock,
  ShieldCheck,
  SlidersHorizontal,
  Trash2,
  User,
  type LucideIcon,
} from "lucide-react";
import { Link } from "@tanstack/react-router";
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
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAssistantWorkspaceCounts } from "@/hooks/use-assistant-workspace";
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
    <div className="px-3 py-2 text-[9px] font-medium uppercase tracking-[1.5px] text-text-tertiary/50">
      {children}
    </div>
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
          className="flex w-full cursor-not-allowed items-center gap-3 rounded-lg px-3 py-2 text-[13px] text-muted-foreground opacity-50"
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
          "w-full overflow-hidden px-3 py-2 text-left text-[13px] transition-colors",
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
        </span>
        {showDraft && (
          <span
            id={draftPreviewId}
            className="mt-0.5 flex min-w-0 items-center gap-1 text-[11px] leading-4 text-text-tertiary"
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
              "absolute right-1 top-1 flex h-6 w-6 items-center justify-center rounded-md bg-card text-muted-foreground shadow-sm outline-none transition-opacity hover:bg-overlay-strong hover:text-foreground",
              "pointer-events-none opacity-0",
              "[@media(hover:none)]:pointer-events-auto [@media(hover:none)]:opacity-100",
              "group-hover:pointer-events-auto group-hover:opacity-100",
              "focus-visible:pointer-events-auto focus-visible:opacity-100",
              "data-[state=open]:pointer-events-auto data-[state=open]:opacity-100",
            )}
          >
            <MoreHorizontal className="h-3.5 w-3.5" />
          </button>
        </DropdownMenuTrigger>
        {/* Above the z-[80] mobile sidebar drawer this can be opened from. */}
        <DropdownMenuContent align="end" className="z-[90] min-w-[160px]">
          {onRequestRename ? (
            <DropdownMenuItem
              disabled={Boolean(conversation.active_turn)}
              onSelect={onRequestRename}
            >
              <PencilLine aria-hidden="true" />
              Rename
            </DropdownMenuItem>
          ) : null}
          <DropdownMenuItem
            disabled={Boolean(conversation.active_turn)}
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
  readonly onNewAgent: () => void;
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
        className="flex w-full items-center gap-1.5 rounded-lg px-3 py-1.5 text-left text-[12px] text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn("h-3 w-3 shrink-0 transition-transform", open && "rotate-90")}
        />
        <span className="min-w-0 flex-1 truncate">{group.label}</span>
        <span className="shrink-0 rounded-md border border-hairline bg-overlay px-1 text-[9px] font-medium leading-4 text-text-tertiary">
          {platform}
        </span>
        {running ? (
          <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
        ) : null}
        <span className="shrink-0 tabular-nums text-[11px]">{count}</span>
      </button>
      {open ? (
        <div className="ml-3 space-y-0.5 border-l border-border/60 pl-1.5">
          {visible.map((thread) => renderThread(thread))}
          {hidden > 0 ? (
            <button
              type="button"
              onClick={() => setShown((value) => value + CHANNEL_THREADS_SHOWN * 4)}
              data-keep-drawer-open=""
              className="w-full rounded-lg px-3 py-1 text-left text-[11px] text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
            >
              Show {String(Math.min(hidden, CHANNEL_THREADS_SHOWN * 4))} more
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
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
  const platforms = [...new Set(agent.channels.map((channel) => channel.platform))];
  const threads = selected ? splitChannelThreads(model.threads) : { own: [], bots: [] };
  const pending = agent.pending_acknowledgements;
  const name = agentTitle(agent);
  const handle = agentHandle(agent);
  // Under the name: NyxBot's role, and the @handle when a display name hides it.
  const subtitle = nyxbot
    ? handle
      ? `${handle} · personal agent`
      : "Your personal agent"
    : handle;
  return (
    <div>
      <button
        type="button"
        onClick={() => model.onSelectAgent(agent.id)}
        aria-label={agentAccessibleName(agent)}
        aria-expanded={selected}
        className={cn(
          "flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-[13px] transition-colors",
          selected
            ? "font-medium text-foreground"
            : "text-muted-foreground hover:bg-overlay hover:text-foreground",
          agent.status === "destroyed" && "opacity-50",
        )}
      >
        <span className="relative flex shrink-0">
          <AgentAvatar agent={agent} size="sm" />
          {nyxbot ? null : (
            <AgentStatusDot
              status={agent.status}
              className="absolute -bottom-0.5 -right-0.5 ring-2 ring-background"
            />
          )}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate">{name}</span>
          {subtitle ? (
            <span className="block truncate text-[10px] font-normal text-text-tertiary">
              {subtitle}
            </span>
          ) : null}
        </span>
        {platforms.slice(0, 1).map((platform) => (
          <span
            key={platform}
            className="shrink-0 rounded-md border border-hairline bg-overlay px-1 text-[9px] font-medium leading-4 text-text-tertiary"
          >
            {channelPlatformName(platform)}
            {platforms.length > 1 ? ` +${String(platforms.length - 1)}` : ""}
          </span>
        ))}
        {pending > 0 ? (
          <span className="shrink-0 rounded-md border border-warning/30 bg-warning/10 px-1.5 text-[10px] font-medium text-warning">
            {pending}
          </span>
        ) : null}
      </button>
      {selected ? (
        <div
          role="group"
          aria-label={`Threads with ${name}`}
          className="mb-1 ml-3 mt-0.5 space-y-0.5 border-l border-border/60 pl-1.5"
        >
          {threads.own.map((conversation) => renderThread(conversation))}
          {model.threadsLoading && !model.threads.length ? (
            <p className="px-3 py-1.5 text-[11px] text-text-tertiary">
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
          {agent.status === "destroyed" || agent.can_use === false ? null : (
            <button
              type="button"
              onClick={() => model.onNewThread(agent.id)}
              aria-label={`New chat with ${name}`}
              className="flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-left text-[12px] text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
            >
              <Plus aria-hidden="true" className="h-3 w-3" />
              New chat
            </button>
          )}
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
        <section key={section.id} aria-label={section.label}>
          <p className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
            {section.label}
          </p>
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
          className="w-full rounded-lg px-3 py-1.5 text-left text-[11px] text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
        >
          {showDestroyed ? "Hide destroyed" : `Show destroyed (${String(destroyed.length)})`}
        </button>
      ) : null}
    </div>
  );
}

function GroupsSection({ model }: { readonly model: SidebarGroups }) {
  return (
    <div className="space-y-0.5">
      {model.groups.map((group) => {
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
              "flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-[13px] transition-colors",
              selected
                ? "bg-overlay-strong font-medium text-foreground"
                : "text-muted-foreground hover:bg-overlay hover:text-foreground",
            )}
          >
            <AgentAvatarStack agents={group.members} size="xs" max={3} />
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
      {model.loading && !model.groups.length ? (
        <p className="px-3 py-1.5 text-[11px] text-text-tertiary">Loading groups...</p>
      ) : null}
      {!model.loading && !model.groups.length ? (
        <button
          type="button"
          onClick={model.onNewGroup}
          data-keep-drawer-open=""
          className="w-full rounded-lg px-3 py-1.5 text-left text-[11px] text-text-tertiary transition-colors hover:bg-overlay hover:text-muted-foreground"
        >
          Chat with several agents at once
        </button>
      ) : null}
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
    | "chat" | "plugins" | "approvals" | "automations" | "machines";
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
  const user = useAuthStore((state) => state.user);
  const counts = useAssistantWorkspaceCounts();
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

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
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
            className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-[13px] transition-colors ${
              agents.homeActive
                ? "bg-overlay-strong font-medium text-foreground"
                : "text-muted-foreground hover:bg-overlay hover:text-foreground"
            }`}
          >
            <House
              className={`h-4 w-4 shrink-0 ${agents.homeActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
            />
            <span className="truncate">Home</span>
          </button>
        ) : (
          <Link
            to="/assistant"
            className="flex items-center gap-3 rounded-lg px-3 py-2 text-[13px] text-muted-foreground hover:bg-overlay hover:text-foreground"
          >
            <House className="h-4 w-4" />
            Home
          </Link>
        )}
        {(
          [
            {
              view: "automations",
              to: "/assistant/automations",
              label: "Automations",
              icon: CalendarClock,
            },
            {
              view: "machines",
              to: "/assistant/machines",
              label: "Machines",
              icon: Monitor,
            },
          ] as const
        ).map((item) => (
          <Link
            key={item.view}
            to={item.to}
            aria-current={activeView === item.view ? "page" : undefined}
            className={cn(
              "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-[13px] transition-colors",
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
          className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-[13px] transition-colors ${
            pluginsActive
              ? "bg-overlay-strong font-medium text-foreground"
              : "text-muted-foreground hover:bg-overlay hover:text-foreground"
          }`}
        >
          <LayoutGrid
            className={`h-4 w-4 shrink-0 ${pluginsActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
          />
          <span className="truncate">Plugins</span>
        </Link>
        <ComingSoonItem
          icon={FileText}
          label="Artifacts"
          trailing={
            <span className="font-mono text-[9px] text-text-tertiary">
              {counts.data?.artifacts ?? 0}
            </span>
          }
        />
        <Link
          to="/assistant/approvals"
          aria-current={approvalsActive ? "page" : undefined}
          className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-[13px] transition-colors ${
            approvalsActive
              ? "bg-overlay-strong font-medium text-foreground"
              : "text-muted-foreground hover:bg-overlay hover:text-foreground"
          }`}
        >
          <ShieldCheck
            className={`h-4 w-4 shrink-0 ${approvalsActive ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
          />
          <span className="min-w-0 flex-1 truncate">Approvals</span>
          {(counts.data?.pendingApprovals ?? 0) > 0 && (
            <span className="rounded-md border border-warning/30 bg-warning/10 px-1.5 text-[10px] font-medium text-warning">
              {counts.data?.pendingApprovals}
            </span>
          )}
        </Link>
        <ComingSoonItem icon={Activity} label="Activity" />
      </div>

      {notice ? (
        <p
          role="status"
          className="mx-2 mb-2 rounded-md border border-border bg-overlay px-2.5 py-2 text-[10px] leading-relaxed text-muted-foreground"
        >
          {notice}
        </p>
      ) : null}
      <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {agents ? (
          <>
            <div className="-ml-2 flex items-center justify-between">
              <GroupLabel>Agents</GroupLabel>
              <button
                type="button"
                aria-label="New agent"
                // The dialog is rendered by this sidebar; closing the mobile
                // drawer would unmount it.
                data-keep-drawer-open=""
                onClick={agents.onNewAgent}
                className="flex h-6 w-6 items-center justify-center rounded-md text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
              >
                <Plus className="h-3.5 w-3.5" />
              </button>
            </div>
            <AgentsSection
              model={agents}
              activeThreadId={activeConversationId}
              renderThread={renderRow}
            />
          </>
        ) : null}
        {groups ? (
          <>
            <div className="-ml-2 mt-2 flex items-center justify-between">
              <GroupLabel>Groups</GroupLabel>
              <button
                type="button"
                aria-label="New group"
                data-keep-drawer-open=""
                onClick={groups.onNewGroup}
                className="flex h-6 w-6 items-center justify-center rounded-md text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground"
              >
                <Plus className="h-3.5 w-3.5" />
              </button>
            </div>
            <GroupsSection model={groups} />
          </>
        ) : null}
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

      <div className="shrink-0 border-t border-border/60 p-2">
        <Link
          to="/dashboard"
          className="flex items-center gap-3 rounded-lg px-3 py-2 text-[13px] text-muted-foreground transition-colors hover:bg-overlay hover:text-foreground"
        >
          <SlidersHorizontal className="h-4 w-4 text-text-tertiary" />
          <span className="flex-1">Studio</span>
          <ChevronRight className="h-3.5 w-3.5" />
        </Link>
        <div className="mt-1 flex items-center gap-3 rounded-lg px-3 py-2">
          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay-strong">
            <User className="h-3.5 w-3.5 text-text-tertiary" />
          </div>
          <div className="min-w-0">
            <p className="truncate text-[12px] font-medium text-foreground">
              {user?.display_name ?? "User"}
            </p>
            <p className="truncate text-[10px] text-text-tertiary">
              {user?.email ?? ""}
            </p>
          </div>
        </div>
      </div>

      {renameTarget && onRename ? (
        <RenameChatDialog
          key={renameTarget.id}
          conversation={renameTarget}
          onClose={() => setRenameTarget(undefined)}
          onRename={onRename}
        />
      ) : null}
      <Dialog
        open={deleteTarget !== undefined}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(undefined);
        }}
      >
        {/* Lifts the panel over the z-[80] mobile sidebar drawer this can be
            opened from. Dialog's own overlay stays at z-50 and so sits under
            that drawer, which only shows during the slide transition -- the
            settled mobile panel is opaque and full-screen. */}
        <DialogContent className="z-[90] md:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete chat?</DialogTitle>
            <DialogDescription>
              &ldquo;{deleteTarget?.title}&rdquo; and its history are removed
              permanently.
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

function RenameChatDialog({
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
    <Dialog open onOpenChange={(open) => {
      if (!open) onClose();
    }}>
      <DialogContent className="z-[90] md:max-w-md">
        <DialogHeader>
          <DialogTitle>Rename chat</DialogTitle>
          <DialogDescription>
            Choose a title for this conversation.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={form.handleSubmit(async ({ title }) => {
          try {
            await onRename(conversation.id, title);
            onClose();
          } catch {
            setError("Could not rename this chat. Try again.");
          }
        })}>
          <label htmlFor="chat-title" className="text-[12px]">
            Title
          </label>
          <Input id="chat-title" maxLength={200} {...form.register("title")} />
          {error ? (
            <p role="alert" className="mt-2 text-[12px] text-destructive">
              {error}
            </p>
          ) : null}
          <DialogFooter className="mt-4">
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
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
