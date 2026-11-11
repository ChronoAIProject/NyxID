import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { CalendarClock, Loader2, UserRound, Bell } from "lucide-react";
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
  NOTIFICATION_EMPTY_PAGE_AUTOLOAD_LIMIT,
  trailingEmptyPages,
  useMarkNotificationsRead,
  useNotificationList,
  useNotificationSummary,
  useNotificationsEnabled,
} from "@/hooks/use-assistant-notifications";
import {
  notificationTarget,
  openNotificationLink,
} from "@/lib/assistant/notification-navigation";
import {
  NOTIFICATION_KIND_PRESENTATION,
  notificationRowLabel,
  notificationSentence,
  notificationFilterCount,
  notificationPlatformSlug,
} from "@/lib/assistant/notification-presentation";
import { AgentAvatar, AgentAvatarStack } from "./nyxbot-agent-avatar";
import { ServiceIcon } from "@/components/service-icon";
import { restorePanelFocus } from "@/lib/assistant/panel-focus";
import { cn, formatRelativeTime } from "@/lib/utils";
import {
  NOTIFICATION_FILTERS,
  NOTIFICATION_FILTER_LABEL,
  notificationFilter,
  type AssistantNotification,
  type NotificationFilter,
} from "@/schemas/assistant-notifications";

const EMPTY_TITLE: Record<NotificationFilter, string> = {
  all: "Nothing new",
  needs_you: "Nothing needs you right now",
  automations: "No automation results yet",
  chats: "No chat updates yet",
};

export function NotificationRow({
  item,
  onOpen,
}: {
  readonly item: AssistantNotification;
  readonly onOpen: (item: AssistantNotification) => void;
}) {
  useUiTranslation();
  const kind = NOTIFICATION_KIND_PRESENTATION[item.kind];
  const Icon = kind.icon;
  const sentence = notificationSentence(item);
  const agents = (item.participants ?? [])
    .filter((actor) => actor.type === "agent" && actor.id)
    .map((actor) => ({
      id: actor.id!,
      name: actor.name,
      display_name: actor.display_name,
      kind: actor.kind ?? ("unknown" as const),
    }));
  const Fallback =
    item.actor?.type === "automation"
      ? CalendarClock
      : item.actor?.type === "person"
        ? UserRound
        : Bell;
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
        <span
          className="relative mt-0.5 shrink-0"
          data-notification-avatar={
            agents.length ? "crew" : (item.actor?.type ?? "system")
          }
        >
          {agents.length ? (
            <AgentAvatarStack agents={agents} size="lg" />
          ) : item.actor?.type === "agent" ? (
            <AgentAvatar
              agent={{
                id: item.actor.id ?? item.source.agent_id ?? item.actor.name,
                name: item.actor.name,
                display_name: item.actor.display_name,
                kind: item.actor.kind ?? "unknown",
              }}
              size="lg"
            />
          ) : (
            <span className="flex h-8 w-8 items-center justify-center rounded-full border border-hairline bg-overlay text-text-tertiary">
              <Fallback aria-hidden="true" className="h-4 w-4" />
            </span>
          )}
          <span
            aria-hidden="true"
            data-notification-badge={item.kind}
            className={cn(
              "absolute -bottom-1 -right-1 flex h-4 w-4 items-center justify-center rounded-full bg-background ring-2 ring-background",
              kind.tone,
            )}
          >
            {item.kind === "chat_update" && item.platform ? (
              <ServiceIcon
                slug={notificationPlatformSlug(item.platform)}
                size="2xs"
              />
            ) : (
              <Icon className="h-3 w-3" />
            )}
          </span>
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span
              className={cn(
                "min-w-0 flex-1",
                unread
                  ? "type-secondary-emphasis text-foreground"
                  : "type-secondary text-muted-foreground",
              )}
            >
              <strong className="type-secondary-emphasis">
                {sentence.actor}
              </strong>
              {sentence.action ? <> {sentence.action}</> : null}
              {sentence.target ? (
                <>
                  {" "}
                  <strong className="type-secondary-emphasis">
                    {sentence.target}
                  </strong>
                </>
              ) : null}
              {sentence.platform ? <> {sentence.platform}</> : null}
            </span>
            {item.count > 1 ? (
              <span className="shrink-0 rounded-md border border-hairline bg-overlay px-1 type-group-label tabular-nums text-text-tertiary">
                {unread
                  ? translateUi("{{count}} new", { count: item.count })
                  : translateUi("{{count}} updates", { count: item.count })}
              </span>
            ) : null}
            {pending ? (
              <span
                className={cn(
                  badgeVariants({ variant: "warning" }),
                  "shrink-0",
                )}
              >
                {translateUi("Needs you")}
              </span>
            ) : null}
          </span>
          {item.preview ? (
            <span className="mt-0.5 line-clamp-2 break-words type-secondary text-text-tertiary">
              {item.preview}
            </span>
          ) : null}
          <span className="mt-0.5 flex min-w-0 items-center gap-1 type-secondary text-text-tertiary">
            <span className="max-w-[70%] truncate rounded-md border border-hairline bg-overlay px-1.5 py-0.5 type-group-label">
              {item.source.label || translateUi(kind.label)}
            </span>
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
  useUiTranslation();
  const { current, update } = useAccountPanel();
  // The account panel unmounts this sheet when the feature goes away; until
  // it does, no further feed requests leave and nothing renders.
  const enabled = useNotificationsEnabled();
  const filter = notificationFilter(current.panelTab) ?? "all";
  const summary = useNotificationSummary(enabled);
  const list = useNotificationList(filter, enabled);
  const markRead = useMarkNotificationsRead();
  const navigate = useNavigate();
  const [announcement, setAnnouncement] = useState<string>();
  const items = useMemo(
    () => list.data?.pages.flatMap((page) => page.items) ?? [],
    [list.data],
  );
  const asOf = list.data?.pages[0]?.as_of;
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = list;
  // A page with no visible rows but a cursor (hidden or revoked sources only)
  // keeps paging reachable; automatic loading stops after a few such pages so
  // a long hidden stretch takes a deliberate Load more rather than a loop.
  const autoLoad =
    trailingEmptyPages(list.data?.pages ?? []) <
    NOTIFICATION_EMPTY_PAGE_AUTOLOAD_LIMIT;
  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = sentinel.current;
    if (
      !element ||
      !hasNextPage ||
      !autoLoad ||
      typeof IntersectionObserver === "undefined"
    )
      return;
    const observer = new IntersectionObserver((entries) => {
      if (
        entries.some((entry) => entry.isIntersecting) &&
        !isFetchingNextPage
      ) {
        void fetchNextPage();
      }
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasNextPage, autoLoad, isFetchingNextPage, fetchNextPage]);

  function selectFilter(value: string) {
    const next = notificationFilter(value) ?? "all";
    if (next === filter) return;
    void update({ panelTab: next === "all" ? undefined : next }, "replace");
  }

  function openItem(item: AssistantNotification) {
    if (!item.read) {
      markRead.mutate(
        { ids: [item.id] },
        {
          onError: () =>
            toast.error(translateUi("Could not mark the notification as read")),
        },
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
        onSuccess: () =>
          setAnnouncement(translateUi("All notifications marked as read.")),
        onError: () =>
          toast.error(translateUi("Could not mark notifications as read")),
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
        aria-label={translateUi("Notifications")}
        className="flex w-full flex-col gap-0 p-0 sm:max-w-lg"
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          restorePanelFocus(opener());
        }}
      >
        <SheetHeader className="shrink-0 space-y-1 px-5 pt-5 pr-14 text-left">
          <SheetTitle>{translateUi("Notifications")}</SheetTitle>
          <SheetDescription>
            {translateUi(
              "Automation results and updates from your chats while you were away.",
            )}
          </SheetDescription>
        </SheetHeader>
        <div className="flex shrink-0 flex-col items-stretch gap-2 px-5 pt-3">
          <SegmentedControl
            value={filter}
            onValueChange={selectFilter}
            className="min-w-0"
          >
            <SegmentedControlList
              aria-label={translateUi("Filter notifications")}
              className="flex-nowrap overflow-x-auto"
            >
              {NOTIFICATION_FILTERS.map((value) => (
                <SegmentedControlTrigger
                  key={value}
                  value={value}
                  aria-label={translateUi(NOTIFICATION_FILTER_LABEL[value])}
                  className="whitespace-nowrap"
                >
                  {translateUi(NOTIFICATION_FILTER_LABEL[value])}
                  {summary.data ? (
                    <span
                      data-filter-count={value}
                      className="ml-1 type-group-label tabular-nums text-text-tertiary"
                    >
                      {notificationFilterCount(summary.data, value)}
                    </span>
                  ) : null}
                </SegmentedControlTrigger>
              ))}
            </SegmentedControlList>
          </SegmentedControl>
        </div>
        <p role="status" aria-live="polite" className="sr-only">
          {announcement}
        </p>
        <div className="assistant-scrollbar min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 pb-4 pt-3">
          {list.isPending ? (
            <p
              role="status"
              className="px-2 py-6 text-center type-secondary text-text-tertiary"
            >
              {translateUi("Loading notifications...")}
            </p>
          ) : list.error ? (
            <div role="alert">
              <ErrorBanner
                message={translateUi(
                  "Could not load notifications. {{error}}",
                  { error: list.error.message },
                )}
                onRetry={() => void list.refetch()}
              />
            </div>
          ) : items.length === 0 && !hasNextPage ? (
            <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
              <MobileNotificationIcon className="h-40 w-40 text-muted-foreground/30" />
              <p className="type-row-title text-text-tertiary">
                {translateUi(EMPTY_TITLE[filter])}
              </p>
              <p className="type-secondary text-text-tertiary">
                {translateUi(
                  "Automation results and updates from your chats show up here.",
                )}
              </p>
            </div>
          ) : (
            <>
              {items.length ? (
                <ul
                  aria-label={translateUi("Notifications")}
                  className="space-y-1"
                >
                  {items.map((item) => (
                    <NotificationRow
                      key={item.id}
                      item={item}
                      onOpen={openItem}
                    />
                  ))}
                </ul>
              ) : (
                // Every row of the pages so far is hidden; older pages may still have some.
                <p
                  role="status"
                  className="px-2 py-6 text-center type-secondary text-text-tertiary"
                >
                  {isFetchingNextPage
                    ? translateUi("Looking further back...")
                    : translateUi("Nothing to show on this page yet.")}
                </p>
              )}
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
                      <Loader2
                        aria-hidden="true"
                        className="h-3.5 w-3.5 animate-spin"
                      />
                    ) : null}
                    {translateUi("Load more")}
                  </Button>
                </div>
              ) : null}
            </>
          )}
        </div>
        <footer className="shrink-0 border-t border-hairline px-5 py-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="w-full"
            onClick={markAll}
            disabled={
              !asOf ||
              (summary.data?.unread_count ?? 0) === 0 ||
              markRead.isPending
            }
          >
            {translateUi("Mark all notifications as read")}
          </Button>
        </footer>
      </SheetContent>
    </Sheet>
  );
}
