import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  AtSign,
  Bot,
  CircleCheck,
  CircleHelp,
  Loader2,
  MessageSquare,
  type LucideIcon,
} from "lucide-react";
import { toast } from "sonner";
import { ErrorBanner } from "@/components/shared/error-banner";
import { MobileNotificationIcon } from "@/components/icons/empty-state";
import { badgeVariants } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  SegmentedControl,
  SegmentedControlList,
  SegmentedControlTrigger,
} from "@/components/ui/segmented-control";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { useAccountPanel } from "@/hooks/use-account-panel";
import {
  useMarkNotificationsRead,
  useNotificationList,
  useNotificationsEnabled,
} from "@/hooks/use-assistant-notifications";
import { notificationTarget, openNotificationLink } from "@/lib/assistant/notification-navigation";
import { restorePanelFocus } from "@/lib/assistant/panel-focus";
import { cn, formatRelativeTime } from "@/lib/utils";
import {
  NOTIFICATION_FILTERS,
  NOTIFICATION_FILTER_LABEL,
  notificationFilter,
  type AssistantNotification,
  type NotificationFilter,
  type NotificationKind,
} from "@/schemas/assistant-notifications";

interface KindPresentation {
  readonly icon: LucideIcon;
  readonly label: string;
  readonly tone: string;
}

/** Semantic colour does the status work; neutral kinds stay quiet. */
export const NOTIFICATION_KIND_PRESENTATION: Record<NotificationKind, KindPresentation> = {
  automation_result: { icon: CircleCheck, label: "Automation finished", tone: "text-success" },
  automation_problem: {
    icon: AlertTriangle,
    label: "Automation needs attention",
    tone: "text-destructive",
  },
  needs_decision: { icon: CircleHelp, label: "Needs your decision", tone: "text-warning" },
  agent_update: { icon: Bot, label: "Agent update", tone: "text-text-tertiary" },
  chat_update: { icon: MessageSquare, label: "Chat update", tone: "text-text-tertiary" },
  group_mention: { icon: AtSign, label: "Mentioned you", tone: "text-text-tertiary" },
};

const EMPTY_TITLE: Record<NotificationFilter, string> = {
  all: "Nothing new",
  needs_you: "Nothing needs you right now",
  automations: "No automation results yet",
  chats: "No chat updates yet",
};

export function notificationRowLabel(item: AssistantNotification): string {
  const kind = NOTIFICATION_KIND_PRESENTATION[item.kind];
  const parts = [item.title || kind.label];
  if (!item.read) parts.push("unread");
  if (item.kind === "needs_decision" && !item.resolved) parts.push("needs your decision");
  return parts.join(", ");
}

function NotificationRow({
  item,
  onOpen,
}: {
  readonly item: AssistantNotification;
  readonly onOpen: (item: AssistantNotification) => void;
}) {
  const kind = NOTIFICATION_KIND_PRESENTATION[item.kind];
  const Icon = kind.icon;
  const unread = !item.read;
  const pending = item.kind === "needs_decision" && !item.resolved;
  return (
    <li>
      <button
        type="button"
        onClick={() => onOpen(item)}
        aria-label={notificationRowLabel(item)}
        data-unread={unread ? "" : undefined}
        className={cn(
          "flex w-full items-start gap-3 rounded-lg px-2 py-2 text-left transition-colors hover:bg-overlay focus-visible:outline-2 focus-visible:outline-ring",
          unread && "bg-overlay/60",
        )}
      >
        <span className={cn("mt-0.5 shrink-0", kind.tone)}>
          <Icon aria-hidden="true" className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            {unread ? (
              <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
            ) : null}
            <span
              className={cn(
                "min-w-0 flex-1 truncate type-row-title",
                unread ? "text-foreground" : "text-muted-foreground",
              )}
            >
              {item.title || kind.label}
            </span>
            {item.count > 1 ? (
              <span className="shrink-0 rounded-md border border-hairline bg-overlay px-1 type-group-label tabular-nums text-text-tertiary">
                {String(item.count)} {unread ? "new" : "updates"}
              </span>
            ) : null}
            {pending ? (
              <span className={cn(badgeVariants({ variant: "warning" }), "shrink-0")}>Needs you</span>
            ) : null}
          </span>
          {item.preview ? (
            <span className="mt-0.5 line-clamp-2 break-words type-secondary text-text-tertiary">
              {item.preview}
            </span>
          ) : null}
          <span className="mt-0.5 flex min-w-0 items-center gap-1 type-secondary text-text-tertiary">
            <span className="truncate">{item.source.label || kind.label}</span>
            <span aria-hidden="true">·</span>
            <time dateTime={item.updated_at} className="shrink-0">
              {formatRelativeTime(item.updated_at)}
            </time>
          </span>
        </span>
      </button>
    </li>
  );
}

/**
 * The Notifications feed: a right-hand sheet opened by `?panel=notifications`.
 * Filters are URL state (`panelTab`, replaced in history like settings tabs),
 * pages load as the list scrolls, and "Mark all as read" is fenced to the
 * moment the first page was read so later arrivals stay unread.
 */
export function NotificationsPanel({
  onClose,
  opener,
}: {
  readonly onClose: () => void;
  readonly opener: () => HTMLElement | null;
}) {
  const { current, update } = useAccountPanel();
  // The account panel unmounts this sheet when the feature goes away; until
  // it does, no further feed requests leave and nothing renders.
  const enabled = useNotificationsEnabled();
  const filter = notificationFilter(current.panelTab) ?? "all";
  const list = useNotificationList(filter, enabled);
  const markRead = useMarkNotificationsRead();
  const navigate = useNavigate();
  const [announcement, setAnnouncement] = useState<string>();
  const items = useMemo(() => list.data?.pages.flatMap((page) => page.items) ?? [], [list.data]);
  const asOf = list.data?.pages[0]?.as_of;
  const loadedUnread = items.some((item) => !item.read);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = list;
  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = sentinel.current;
    if (!element || !hasNextPage || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting) && !isFetchingNextPage) {
        void fetchNextPage();
      }
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  function selectFilter(value: string) {
    const next = notificationFilter(value) ?? "all";
    if (next === filter) return;
    void update({ panelTab: next === "all" ? undefined : next }, "replace");
  }

  function openItem(item: AssistantNotification) {
    if (!item.read) {
      markRead.mutate(
        { ids: [item.id] },
        { onError: () => toast.error("Could not mark the notification as read") },
      );
    }
    const target = notificationTarget(item, window.location.origin);
    if (!target) return;
    if (target.kind === "link") {
      openNotificationLink(target.href);
      return;
    }
    void navigate({ to: target.to as never, search: target.search as never });
  }

  if (!enabled) return null;

  function markAll() {
    if (!asOf) return;
    markRead.mutate(
      { all: true, before: asOf },
      {
        onSuccess: () => setAnnouncement("All notifications marked as read."),
        onError: () => toast.error("Could not mark notifications as read"),
      },
    );
  }

  return (
    <Sheet
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <SheetContent
        side="right"
        aria-label="Notifications"
        className="flex w-full flex-col gap-0 p-0 sm:max-w-lg"
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          restorePanelFocus(opener());
        }}
      >
        <SheetHeader className="shrink-0 space-y-1 px-5 pt-5 pr-14 text-left">
          <SheetTitle>Notifications</SheetTitle>
          <SheetDescription>
            Automation results and updates from your chats while you were away.
          </SheetDescription>
        </SheetHeader>
        <div className="flex shrink-0 flex-wrap items-center gap-2 px-5 pt-3">
          <SegmentedControl value={filter} onValueChange={selectFilter} className="min-w-0 flex-1">
            <SegmentedControlList aria-label="Filter notifications">
              {NOTIFICATION_FILTERS.map((value) => (
                <SegmentedControlTrigger key={value} value={value}>
                  {NOTIFICATION_FILTER_LABEL[value]}
                </SegmentedControlTrigger>
              ))}
            </SegmentedControlList>
          </SegmentedControl>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={markAll}
            disabled={!asOf || !loadedUnread || markRead.isPending}
          >
            Mark all as read
          </Button>
        </div>
        <p role="status" aria-live="polite" className="sr-only">
          {announcement}
        </p>
        <div className="assistant-scrollbar min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 pb-4 pt-3">
          {list.isPending ? (
            <p role="status" className="px-2 py-6 text-center type-secondary text-text-tertiary">
              Loading notifications...
            </p>
          ) : list.error ? (
            <div role="alert">
              <ErrorBanner
                message={`Could not load notifications. ${list.error.message}`}
                onRetry={() => void list.refetch()}
              />
            </div>
          ) : items.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
              <MobileNotificationIcon className="h-40 w-40 text-muted-foreground/30" />
              <p className="type-row-title text-text-tertiary">{EMPTY_TITLE[filter]}</p>
              <p className="type-secondary text-text-tertiary">
                Automation results and updates from your chats show up here.
              </p>
            </div>
          ) : (
            <>
              <ul aria-label="Notifications" className="space-y-1">
                {items.map((item) => (
                  <NotificationRow key={item.id} item={item} onOpen={openItem} />
                ))}
              </ul>
              <div ref={sentinel} aria-hidden="true" className="h-px" />
              {hasNextPage ? (
                <div className="flex justify-center pt-2">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={isFetchingNextPage}
                    onClick={() => void fetchNextPage()}
                  >
                    {isFetchingNextPage ? (
                      <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
                    ) : null}
                    Load more
                  </Button>
                </div>
              ) : null}
            </>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
