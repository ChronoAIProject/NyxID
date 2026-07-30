import { useState, type ReactNode } from "react";
import {
  Activity,
  ChevronRight,
  FileText,
  LayoutGrid,
  MoreHorizontal,
  Plus,
  Server,
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
import { useWorkspaceCounts } from "@/hooks/use-assistant";
import { cn } from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";
import type { Conversation } from "@/types/assistant";

/**
 * Titles run to the very end of the column, so whenever the 3-dot is showing
 * it would otherwise sit on top of the text. Masking the tail (rather than
 * covering it with a chip) keeps the fade correct on every row background --
 * default, hover and active are all translucent overlays over `background`,
 * so no single gradient colour would match all three.
 */
const TITLE_FADE =
  "[mask-image:linear-gradient(to_right,#000_calc(100%_-_2.5rem),transparent)]";

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
  onSelect,
  onRequestDelete,
}: {
  readonly conversation: Conversation;
  readonly active: boolean;
  readonly onSelect: () => void;
  readonly onRequestDelete: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
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
        className={cn(
          "w-full truncate px-3 py-2 text-left text-[13px] transition-colors",
          active
            ? "font-medium text-foreground"
            : "text-muted-foreground group-hover:text-foreground",
          "[@media(hover:none)]:[mask-image:linear-gradient(to_right,#000_calc(100%_-_2.5rem),transparent)]",
          "group-hover:[mask-image:linear-gradient(to_right,#000_calc(100%_-_2.5rem),transparent)]",
          menuOpen && TITLE_FADE,
        )}
      >
        {conversation.title}
      </button>
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={`Options for ${conversation.title}`}
            data-keep-drawer-open=""
            className={cn(
              "absolute right-1 top-1/2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-md text-text-tertiary transition-opacity hover:bg-overlay-strong hover:text-foreground",
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
  readonly onDelete: (conversationId: string) => void | Promise<void>;
}) {
  const user = useAuthStore((state) => state.user);
  const counts = useWorkspaceCounts();
  const pluginsActive = activeView === "plugins";
  const approvalsActive = activeView === "approvals";
  const [deleteTarget, setDeleteTarget] = useState<Conversation | undefined>();
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
              onSelect={() => onSelect(conversation.id)}
              onRequestDelete={() => setDeleteTarget(conversation)}
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
