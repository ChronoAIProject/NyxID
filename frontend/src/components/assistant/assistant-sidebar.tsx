import type { ReactNode } from "react";
import {
  Activity,
  ChevronRight,
  FileText,
  LayoutGrid,
  MessageSquare,
  Plus,
  Server,
  ShieldCheck,
  SlidersHorizontal,
  User,
  type LucideIcon,
} from "lucide-react";
import { Link } from "@tanstack/react-router";
import { TransportToggle } from "@/components/assistant/transport-toggle";
import { Button } from "@/components/ui/button";
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

export function AssistantSidebar({
  conversations,
  activeConversationId,
  activeView = "chat",
  creating,
  turnActive = false,
  onNewChat,
  onSelect,
}: {
  readonly conversations: readonly Conversation[];
  readonly activeConversationId: string | undefined;
  readonly activeView?: "chat" | "plugins" | "approvals";
  readonly creating: boolean;
  readonly turnActive?: boolean;
  readonly onNewChat: () => void;
  readonly onSelect: (conversationId: string) => void;
}) {
  const user = useAuthStore((state) => state.user);
  const counts = useWorkspaceCounts();
  const pluginsActive = activeView === "plugins";
  const approvalsActive = activeView === "approvals";

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div className="p-2.5">
        <Button
          type="button"
          variant="primary"
          className="w-full"
          isLoading={creating}
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
          {conversations.map((conversation) => {
            const active = conversation.id === activeConversationId;
            return (
              <button
                key={conversation.id}
                type="button"
                onClick={() => onSelect(conversation.id)}
                className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-[13px] transition-colors ${
                  active
                    ? "bg-overlay-strong font-medium text-foreground"
                    : "text-muted-foreground hover:bg-overlay hover:text-foreground"
                }`}
              >
                <MessageSquare
                  className={`h-4 w-4 shrink-0 ${active ? "text-nyx-secondary-400" : "text-text-tertiary"}`}
                />
                <span className="truncate">{conversation.title}</span>
              </button>
            );
          })}
        </div>
      </nav>

      <div className="shrink-0 border-t border-border/60 p-2">
        <TransportToggle turnActive={turnActive} />
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
