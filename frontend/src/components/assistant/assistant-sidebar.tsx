import { useState, type ReactNode } from "react";
import {
  Activity,
  ChevronRight,
  FileText,
  LayoutGrid,
  MessageSquare,
  MoreHorizontal,
  Plus,
  Server,
  ShieldCheck,
  SlidersHorizontal,
  User,
  type LucideIcon,
} from "lucide-react";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useWorkspaceCounts } from "@/hooks/use-assistant";
import { useAuthStore } from "@/stores/auth-store";
import type { Conversation } from "@/types/assistant";

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
 * One sidebar chat row: the select button plus a hover-revealed 3-dot menu
 * whose popover doubles as the delete confirmation (deleting removes the
 * actor and its history permanently, so a plain one-click delete is too
 * easy to hit by accident). On success the row unmounts with the list; on
 * failure the popover stays open so the action remains retryable.
 */
function ConversationRow({
  conversation,
  active,
  deleting,
  onSelect,
  onDelete,
}: {
  readonly conversation: Conversation;
  readonly active: boolean;
  readonly deleting: boolean;
  readonly onSelect: () => void;
  readonly onDelete: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  return (
    <div
      className={`group relative flex items-center rounded-lg transition-colors ${
        active ? "bg-overlay-strong" : "hover:bg-overlay"
      }`}
    >
      <button
        type="button"
        onClick={onSelect}
        className={`flex min-w-0 flex-1 items-center gap-3 px-3 py-2 text-left text-[13px] transition-colors ${
          active
            ? "font-medium text-foreground"
            : "text-muted-foreground group-hover:text-foreground"
        }`}
      >
        <MessageSquare
          className={`h-4 w-4 shrink-0 ${active ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
        />
        <span className="truncate">{conversation.title}</span>
      </button>
      <Popover open={menuOpen} onOpenChange={setMenuOpen}>
        <PopoverTrigger asChild>
          <button
            type="button"
            aria-label={`Options for ${conversation.title}`}
            className={`mr-1 flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-text-tertiary transition-opacity hover:bg-overlay-strong hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 ${
              menuOpen ? "opacity-100" : "opacity-0"
            }`}
          >
            <MoreHorizontal className="h-3.5 w-3.5" />
          </button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-60 p-3">
          <p className="text-[13px] font-medium text-foreground">
            Delete this chat?
          </p>
          <p className="mt-1 text-[12px] text-muted-foreground">
            The conversation and its history are removed permanently.
          </p>
          <div className="mt-3 flex justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => setMenuOpen(false)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant="destructive"
              size="sm"
              isLoading={deleting}
              onClick={onDelete}
            >
              Delete
            </Button>
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}

export function AssistantSidebar({
  conversations,
  activeConversationId,
  activeView = "chat",
  deletingId,
  onNewChat,
  onSelect,
  onDelete,
}: {
  readonly conversations: readonly Conversation[];
  readonly activeConversationId: string | undefined;
  readonly activeView?: "chat" | "plugins" | "approvals";
  readonly deletingId?: string;
  readonly onNewChat: () => void;
  readonly onSelect: (conversationId: string) => void;
  readonly onDelete: (conversationId: string) => void;
}) {
  const user = useAuthStore((state) => state.user);
  const counts = useWorkspaceCounts();
  const pluginsActive = activeView === "plugins";
  const approvalsActive = activeView === "approvals";

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
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

      <GroupLabel>Workspace</GroupLabel>
      <div className="space-y-0.5 px-2">
        <Link
          to="/assistant/plugins"
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
        <ComingSoonItem icon={Server} label="Devices & Nodes" />
        <ComingSoonItem icon={Activity} label="Activity" />
      </div>

      <GroupLabel>Chats</GroupLabel>
      <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        <div className="space-y-0.5">
          {conversations.map((conversation) => (
            <ConversationRow
              key={conversation.id}
              conversation={conversation}
              active={conversation.id === activeConversationId}
              deleting={conversation.id === deletingId}
              onSelect={() => onSelect(conversation.id)}
              onDelete={() => onDelete(conversation.id)}
            />
          ))}
        </div>
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
    </div>
  );
}
